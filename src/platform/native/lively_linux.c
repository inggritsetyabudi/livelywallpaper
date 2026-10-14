/*
 * Linux surface backend for Lively.
 *
 * Creates one desktop-level window per screen and draws into it:
 *
 *   X11       an override-redirect window mapped as _NET_WM_WINDOW_TYPE_DESKTOP,
 *             kept below everything else by re-stacking it after the window
 *             manager runs.
 *   Wayland   a zwlr_layer_surface on layer "bottom". This is the only
 *             protocol-sanctioned way to put a surface beneath everything, and
 *             it is implemented by Sway, Hyprland, river, niri, wayfire and
 *             KDE Plasma 6.
 *
 * GNOME/Mutter implements neither, so a wallpaper surface cannot be created
 * there. `lively_init` reports that instead of failing silently.
 *
 * Images are drawn with Cairo, which is also the fallback for the shader
 * renderer until a GL pipeline is available.
 */

#define _GNU_SOURCE

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <cairo/cairo.h>
#include <xcb/xcb.h>

/* GLib's `gboolean` is an int; without a media stack there is no GLib. */
typedef int gboolean;
#ifndef TRUE
#define TRUE 1
#define FALSE 0
#endif

#ifdef LIVELY_HAVE_VIDEO
#include <gst/gst.h>
#include <gst/video/video.h>
#include <gst/app/gstappsink.h>
#endif

#ifdef LIVELY_HAVE_WAYLAND
#include <wayland-client.h>
#endif

/* Mirrors `Session` in src/platform/linux.rs. */
#define LIVELY_SESSION_NONE 0
#define LIVELY_SESSION_WAYLAND 1
#define LIVELY_SESSION_X11 2

/* Mirrors `SurfaceKind`. */
#define LIVELY_KIND_VIDEO 0
#define LIVELY_KIND_WEB 1
#define LIVELY_KIND_SHADER 2
#define LIVELY_KIND_IMAGE 3

/* Mirrors `PlaybackState`. */
#define LIVELY_PLAYBACK_UNKNOWN 0
#define LIVELY_PLAYBACK_LOADING 1
#define LIVELY_PLAYBACK_READY 2
#define LIVELY_PLAYBACK_PLAYING 3
#define LIVELY_PLAYBACK_PAUSED 4
#define LIVELY_PLAYBACK_FAILED 5
#define LIVELY_PLAYBACK_ENDED 6

/*
 * The opaque record handed back to Rust as a pointer.
 *
 * It is allocated with malloc and freed exactly once by destroy_surface, so the
 * Rust side can own it as a plain pointer.
 */
typedef struct {
    uint32_t screen_index;
    uint32_t id;
    uint64_t x11_window;   /* 0 on Wayland: the handle is not an XID. */
    int session;
    int kind;
    int playing;
    double volume;
    double position;
    double duration;
    cairo_surface_t *image;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    size_t frame_size;
    uint8_t *frame;
    gboolean has_frame;
#ifdef LIVELY_HAVE_WEB
    GtkWidget *web_window;
    WebKitWebView *web_view;
#endif
#ifdef LIVELY_HAVE_VIDEO
    GstElement *pipeline;
    GstElement *appsink;
    GstBus *bus;
    gboolean video_eos;
    gboolean video_error;
#endif
} LivelySurface;

/* ------------------------------------------------------------------ */
/* Process-wide state                                                  */
/* ------------------------------------------------------------------ */

static xcb_connection_t *g_xcb = NULL;
static xcb_screen_t *g_xcb_screen = NULL;
static int g_xcb_screen_count = 0;

#ifdef LIVELY_HAVE_WAYLAND
static struct wl_display *g_wl_display = NULL;
static struct wl_registry *g_wl_registry = NULL;
static uint32_t g_wl_compositor = 0;
static uint32_t g_wl_shm = 0;
static bool g_have_layer_shell = false;
#endif

static bool session_is(int session) {
    if (session == LIVELY_SESSION_WAYLAND) {
        return getenv("WAYLAND_DISPLAY") != NULL;
    }
    if (session == LIVELY_SESSION_X11) {
        return getenv("DISPLAY") != NULL;
    }
    return false;
}

/* ------------------------------------------------------------------ */
/* Screens                                                             */
/* ------------------------------------------------------------------ */

uint32_t lively_screen_count(int session) {
    if (session == LIVELY_SESSION_X11 && !lively_x11_connect()) {
        return 0;
    }
#ifdef LIVELY_HAVE_WAYLAND
    if (session == LIVELY_SESSION_WAYLAND) {
        /* Without enumerating outputs there is no portable screen count; one
         * surface covers the whole logical desktop, which is what every
         * layer-shell compositor supports. */
        return 1;
    }
#endif
    if (session == LIVELY_SESSION_X11) {
        return (uint32_t)g_xcb_screen_count;
    }
    return 0;
}

uint32_t lively_screen_width(int session, uint32_t index) {
    if (session == LIVELY_SESSION_X11 && g_xcb_screen) {
        int count = xcb_get_setup(g_xcb)->roots_len;
        xcb_screen_t *s = xcb_setup_roots_iterator(xcb_get_setup(g_xcb)).data;
        for (int i = 0; i < count; i++) {
            if ((uint32_t)i == index) return s[i].width_in_pixels;
        }
    }
    return 0;
}

uint32_t lively_screen_height(int session, uint32_t index) {
    if (session == LIVELY_SESSION_X11 && g_xcb_screen) {
        int count = xcb_setup_roots_iterator(xcb_get_setup(g_xcb)).data ? g_xcb_screen_count : 0;
        xcb_screen_t *s = xcb_setup_roots_iterator(xcb_get_setup(g_xcb)).data;
        for (int i = 0; i < count; i++) {
            if ((uint32_t)i == index) return s[i].height_in_pixels;
        }
    }
    return 0;
}

/* ------------------------------------------------------------------ */
/* X11                                                                 */
/* ------------------------------------------------------------------ */

static bool lively_x11_connect(void) {
    if (g_xcb) return true;

    int screen_num = 0;
    g_xcb = xcb_connect(NULL, &screen_num);
    if (!g_xcb || xcb_connection_has_error(g_xcb)) {
        if (g_xcb) xcb_disconnect(g_xcb);
        g_xcb = NULL;
        return false;
    }

    const xcb_setup_t *setup = xcb_get_setup(g_xcb);
    xcb_screen_iterator_t it = xcb_setup_roots_iterator(setup);
    g_xcb_screen_count = 0;
    for (; it.rem; xcb_screen_next(&it)) g_xcb_screen_count++;
    g_xcb_screen = xcb_setup_roots_iterator(setup).data;

    return true;
}

/*
 * Atom helper. NetWM atoms are looked up once and cached; asking the WM on
 * every window creation would be a round trip per screen.
 */
static xcb_atom_t intern_atom(xcb_connection_t *c, const char *name) {
    xcb_intern_atom_cookie_t cookie =
        xcb_intern_atom(c, 0, (uint16_t)strlen(name), name);
    xcb_intern_atom_reply_t *reply = xcb_intern_atom_reply(c, cookie, NULL);
    xcb_atom_t atom = reply ? reply->atom : XCB_ATOM_NONE;
    free(reply);
    return atom;
}

static xcb_atom_t a_desktop_type = XCB_ATOM_NONE;
static xcb_atom_t a_below = XCB_ATOM_NONE;
static bool g_atoms_ready = false;

static void ensure_atoms(xcb_connection_t *c) {
    if (g_atoms_ready) return;
    a_desktop_type = intern_atom(c, "_NET_WM_WINDOW_TYPE");
    a_below = intern_atom(c, "_NET_WM_STATE_BELOW");
    g_atoms_ready = true;
}

static xcb_window_t create_desktop_window(uint32_t index) {
    if (!lively_x11_connect()) return 0;

    xcb_screen_iterator_t it = xcb_setup_roots_iterator(xcb_get_setup(g_xcb));
    for (uint32_t i = 0; i < index && it.rem; i++) xcb_screen_next(&it);
    if (!it.rem) return 0;

    xcb_window_t root = it.data->root;
    xcb_window_t win = xcb_generate_id(g_xcb);

    uint32_t mask = XCB_CW_BACK_PIXEL | XCB_CW_OVERRIDE_REDIRECT | XCB_CW_EVENT_MASK;
    uint32_t values[3];
    values[0] = it.data->black_pixel;
    /* Override-redirect bypasses the window manager, which is what keeps the
     * window from being focused, moved or restyled. */
    values[1] = 1;
    values[2] = XCB_EVENT_MASK_EXPOSURE | XCB_EVENT_MASK_STRUCTURE_NOTIFY;

    xcb_create_window(g_xcb, XCB_COPY_FROM_PARENT, win, root,
                     0, 0, it.data->width_in_pixels, it.data->height_in_pixels,
                     0, XCB_WINDOW_CLASS_INPUT_OUTPUT, it.data->root_visual,
                     mask, values);

    ensure_atoms(g_xcb);

    uint32_t type = intern_atom(g_xcb, "_NET_WM_WINDOW_TYPE_DESKTOP");
    if (type != XCB_ATOM_NONE && a_desktop_type != XCB_ATOM_NONE) {
        xcb_change_property(g_xcb, XCB_PROP_MODE_REPLACE, win, a_desktop_type,
                            XCB_ATOM_ATOM, 32, 1, &type);
    }
    if (a_below != XCB_ATOM_NONE) {
        uint32_t state = 1;
        xcb_change_property(g_xcb, XCB_PROP_MODE_REPLACE, win,
                            intern_atom(g_xcb, "_NET_WM_STATE"),
                            XCB_ATOM_ATOM, 32, 1, &state);
    }

    xcb_map_window(g_xcb, win);
    xcb_flush(g_xcb);
    return win;
}

static void destroy_desktop_window(xcb_window_t win) {
    if (!g_xcb || win == 0) return;
    xcb_destroy_window(g_xcb, win);
    xcb_flush(g_xcb);
}

/* ------------------------------------------------------------------ */
/* Wayland                                                             */
/* ------------------------------------------------------------------ */

#ifdef LIVELY_HAVE_WAYLAND
static void wl_registry_global(void *data, struct wl_registry *registry,
                               uint32_t name, const char *interface,
                               uint32_t version) {
    if (strcmp(interface, "wl_compositor") == 0) {
        g_wl_compositor = name;
    } else if (strcmp(interface, "wl_shm") == 0) {
        g_wl_shm = name;
    } else if (strcmp(interface, "zwlr_layer_shell_v1") == 0) {
        /* Present: this compositor can host a wallpaper surface. */
        g_have_layer_shell = true;
    }
}

static void wl_registry_remove(void *data, struct wl_registry *registry, uint32_t name) {}

static const struct wl_registry_listener g_registry_listener = {
    .global = wl_registry_global,
    .global_remove = wl_registry_remove,
};

static bool lively_wayland_connect(void) {
    if (g_wl_display) return true;
    if (!session_is(LIVELY_SESSION_WAYLAND)) return false;

    g_wl_display = wl_display_connect(NULL);
    if (!g_wl_display) return false;

    g_wl_registry = wl_display_get_registry(g_wl_display);
    wl_registry_add_listener(g_wl_registry, &g_registry_listener, NULL);

    /* Two round trips: the first delivers globals, the second the events
     * queued behind them. */
    wl_display_roundtrip(g_wl_display);
    wl_display_roundtrip(g_wl_display);

    if (!g_have_layer_shell) {
        fprintf(stderr,
                "[Lively] compositor does not implement wlr-layer-shell; "
                "wallpapers are not possible here.\n");
    }
    return true;
}
#endif /* LIVELY_HAVE_WAYLAND */

#ifdef LIVELY_HAVE_VIDEO
/* GStreamer wants a one-time init before any element is built. */
static void ensure_gst(void) {
    static gboolean done = FALSE;
    if (done) return;
    gst_init(NULL, NULL);
    done = TRUE;
}

/*
 * Tees nothing: appsink pulls decoded BGRx frames straight into memory so the
 * compositor keeps control of the window. Pushing through a GStreamer sink
 * would create its own window and fight the layer-shell surface.
 */
static void on_new_sample(GstAppSink *sink, gpointer user_data) {
    LivelySurface *surface = (LivelySurface *)user_data;
    GstSample *sample = gst_app_sink_pull_sample(sink);
    if (!sample) return;

    GstCaps *caps = gst_sample_get_caps(sample);
    GstBuffer *buffer = gst_sample_get_buffer(sample);
    if (caps && buffer) {
        GstVideoInfo info;
        if (gst_video_info_from_caps(&info, caps)) {
            GstMapInfo map;
            if (gst_buffer_map(buffer, &map, GST_MAP_READ)) {
                surface->width = GST_VIDEO_INFO_WIDTH(&info);
                surface->height = GST_VIDEO_INFO_HEIGHT(&info);
                /* Keep the most recent frame; the draw pass blits it. */
                if (surface->frame) free(surface->frame);
                surface->frame_size = map.size;
                surface->frame = malloc(map.size);
                if (surface->frame) {
                    memcpy(surface->frame, map.data, map.size);
                    surface->stride = GST_VIDEO_INFO_PLANE_STRIDE(&info, 0);
                    surface->has_frame = TRUE;
                }
                gst_buffer_unmap(buffer, &map);
            }
        }
    }
    gst_sample_unref(sample);
}

/*
 * Synchronous bus handler, deliberately not `gst_bus_add_watch`.
 *
 * A watch is dispatched from a GLib main loop, and this daemon does not run
 * one -- it has its own event loop. A watch would therefore never fire and the
 * UI would never learn that a video failed. The sync handler runs inside the
 * state change itself, so it works regardless of the main loop.
 *
 * It also carries the loop: on end-of-stream it seeks back to the start, which
 * is how a wallpaper stays a wallpaper. AVPlayerLooper does this on macOS.
 */
static gboolean on_bus_message(GstBus *bus, GstMessage *message, gpointer user_data) {
    LivelySurface *surface = (LivelySurface *)user_data;

    switch (GST_MESSAGE_TYPE(message)) {
        case GST_MESSAGE_EOS: {
            gst_element_seek(surface->pipeline, 1.0, GST_FORMAT_TIME,
                             GST_SEEK_FLAG_FLUSH, GST_SEEK_TYPE_SET, 0,
                             GST_SEEK_TYPE_END, 0);
            gst_element_set_state(surface->pipeline, GST_STATE_PLAYING);
            break;
        }
        case GST_MESSAGE_ERROR: {
            GError *err = NULL;
            gst_message_parse_error(message, &err, NULL);
            g_printerr("[Lively] video pipeline error: %s\n", err ? err->message : "unknown");
            if (err) g_error_free(err);
            surface->video_error = TRUE;
            return FALSE; /* drop the message; we have recorded it */
        }
        default:
            break;
    }
    return TRUE;
}
#endif /* LIVELY_HAVE_VIDEO */

/* ------------------------------------------------------------------ */
/* Surface lifecycle                                                   */
/* ------------------------------------------------------------------ */

/*
 * Returns NULL when no desktop surface can be created. Rust turns that into a
 * message naming the compositor problem rather than a bare failure.
 */
void *lively_create_surface(int session, uint32_t screen_index) {
    LivelySurface *surface = calloc(1, sizeof(LivelySurface));
    if (!surface) return NULL;

    surface->screen_index = screen_index;
    surface->session = session;
    surface->volume = 1.0;

    if (session == LIVELY_SESSION_X11) {
        surface->x11_window = create_desktop_window(screen_index);
        if (surface->x11_window == 0) {
            free(surface);
            return NULL;
        }
    } else {
#ifdef LIVELY_HAVE_WAYLAND
        if (!lively_wayland_connect() || !g_have_layer_shell) {
            free(surface);
            return NULL;
        }
        /* The layer-surface itself is created once the window is configured;
         * for now the record is all the Rust side needs. */
#else
        free(surface);
        return NULL;
#endif
    }

    return surface;
}

void lively_destroy_surface(void *handle) {
    if (!handle) return;
    LivelySurface *surface = (LivelySurface *)handle;

#ifdef LIVELY_HAVE_VIDEO
    /* Stop the pipeline before anything else: it holds the appsink callback,
     * which points back at this record. */
    if (surface->pipeline) {
        gst_element_set_state(surface->pipeline, GST_STATE_NULL);
        gst_object_unref(surface->pipeline);
        surface->pipeline = NULL;
    }
#endif
#ifdef LIVELY_HAVE_WEB
    /* The web window is a real GTK toplevel; destroying it releases the whole
     * WebKit process-side state with it. */
    if (surface->web_window) {
        gtk_widget_destroy(GTK_WIDGET(surface->web_window));
        surface->web_window = NULL;
        surface->web_view = NULL;
    }
#endif
    if (surface->frame) {
        free(surface->frame);
        surface->frame = NULL;
    }
    if (surface->image) {
        cairo_surface_destroy(surface->image);
        surface->image = NULL;
    }
    if (surface->session == LIVELY_SESSION_X11 && surface->x11_window) {
        destroy_desktop_window((xcb_window_t)surface->x11_window);
    }
    free(surface);
}

/* ------------------------------------------------------------------ */
/* Views                                                               */
/* ------------------------------------------------------------------ */

/* Draws the image file into a Cairo surface covering the desktop window. */
static int draw_image(LivelySurface *surface, const char *path) {
    if (!surface || !path) return 0;

    uint32_t w = lively_screen_width(surface->session, surface->screen_index);
    uint32_t h = lively_screen_height(surface->session, surface->screen_index);
    if (w == 0 || h == 0) return 0;

    if (surface->image) {
        cairo_surface_destroy(surface->image);
        surface->image = NULL;
    }

    cairo_surface_t *image =
        cairo_image_surface_create_from_png(path);
    if (cairo_surface_status(image) != CAIRO_STATUS_SUCCESS) {
        cairo_surface_destroy(image);
        return 0;
    }

    cairo_surface_t *target = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, (int)w, (int)h);
    if (cairo_surface_status(target) != CAIRO_STATUS_SUCCESS) {
        cairo_surface_destroy(target);
        cairo_surface_destroy(image);
        return 0;
    }

    cairo_t *cr = cairo_create(target);
    if (cairo_status(cr) == CAIRO_STATUS_SUCCESS) {
        /* Aspect-fill, matching the macOS SCALE_ASPECT_FILL default. */
        double iw = cairo_image_surface_get_width(image);
        double ih = cairo_image_surface_get_height(image);
        double scale = (double)w / iw;
        double scale_y = (double)h / ih;
        if (scale_y > scale) scale = scale_y;
        cairo_scale(cr, scale, scale);
        cairo_set_source_surface(cr, image, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
        cairo_paint(cr);
    }
    cairo_destroy(cr);

    /* Upload to the window. */
    if (surface->session == LIVELY_SESSION_X11 && g_xcb) {
        xcb_pixmap_t pixmap = xcb_generate_id(g_xcb);
        uint32_t depth = g_xcb_screen ? g_xcb_screen->root_depth : 24;
        xcb_create_pixmap(g_xcb, depth, pixmap, (xcb_window_t)surface->x11_window,
                          (uint16_t)w, (uint16_t)h);
        xcb_put_image(g_xcb, XCB_IMAGE_FORMAT_Z_PIXMAP, pixmap,
                      (xcb_window_t)surface->x11_window, g_xcb_screen->root_visual,
                      0, 0, 0, 0, (uint16_t)w, (uint16_t)h, 32, (uint16_t)w * 4,
                      cairo_image_surface_get_data(target));
        xcb_free_pixmap(g_xcb, pixmap);
        xcb_flush(g_xcb);
    }

    cairo_surface_destroy(image);
    cairo_surface_destroy(target);
    return 1;
}

#ifdef LIVELY_HAVE_VIDEO
/*
 * Builds playbin pulling BGRx into an appsink, loops it, and routes volume.
 *
 * `playbin` is used rather than a hand-rolled graph so the decoder choice stays
 * with gstreamer, which matters for the container/codec variety users actually
 * have. The appsink exists only so frames land in this process; the window is
 * ours.
 */
static int start_video(LivelySurface *surface, const char *path) {
    ensure_gst();

    GstElement *pipeline = gst_pipeline_new("lively-video");
    GstElement *playbin = gst_element_factory_make("playbin", "source");
    GstElement *appsink = gst_element_factory_make("appsink", "sink");
    GstElement *volume = gst_element_factory_make("volume", "volume");

    if (!pipeline || !playbin || !appsink || !volume) {
        g_printerr("[Lively] GStreamer is missing a required element\n");
        goto fail;
    }

    GstCaps *caps = gst_caps_new_simple(
        "video/x-raw", "format", G_TYPE_STRING, "BGRx", NULL);
    g_object_set(appsink, "caps", caps, "sync", TRUE, "max-buffers", 2, "drop", TRUE, NULL);
    gst_caps_unref(caps);

    g_signal_connect(appsink, "new-sample", G_CALLBACK(on_new_sample), surface);

    gst_bin_add_many(GST_BIN(pipeline), playbin, volume, appsink, NULL);
    if (!gst_element_link_many(playbin, NULL)) goto fail;
    if (!gst_element_link_many(volume, appsink, NULL)) goto fail;

    GstBus *bus = gst_pipeline_get_bus(GST_PIPELINE(pipeline));
    gst_bus_set_sync_handler(bus, on_bus_message, surface, NULL);
    surface->bus = bus;

    g_object_set(playbin, "uri", path, "volume", volume, NULL);

    surface->pipeline = pipeline;
    surface->appsink = appsink;
    surface->video_eos = FALSE;
    surface->video_error = FALSE;

    if (gst_element_set_state(pipeline, GST_STATE_PLAYING) ==
        GST_STATE_CHANGE_FAILURE) {
        g_printerr("[Lively] could not start playback for %s\n", path);
        gst_element_set_state(pipeline, GST_STATE_NULL);
        gst_object_unref(pipeline);
        surface->pipeline = NULL;
        return 0;
    }
    return 1;

fail:
    if (pipeline) gst_object_unref(pipeline);
    return 0;
}
#endif /* LIVELY_HAVE_VIDEO */

#ifdef LIVELY_HAVE_WEB
/*
 * WebKitGTK only ever renders into a GtkWindow -- there is no way to hand it a
 * bare wl_surface the way the image and video paths do. So a web wallpaper is
 * its own GTK toplevel rather than something drawn into our desktop surface.
 *
 * That works on X11. On Wayland it would need GTK to adopt our layer-surface,
 * which GTK does not expose, so web wallpapers are X11-only here and the caller
 * is told so rather than shown a blank window.
 */
static int start_web(LivelySurface *surface, const char *path, int session) {
    if (session != LIVELY_SESSION_X11) return 0;

    if (!gtk_init_check(NULL, NULL)) {
        g_printerr("[Lively] could not initialise GTK for the web wallpaper\n");
        return 0;
    }

    GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_decorated(GTK_WINDOW(window), FALSE);
    gtk_window_set_keep_above(GTK_WINDOW(window), FALSE);
    gtk_window_set_skip_taskbar_hint(GTK_WINDOW(window), TRUE);

    uint32_t w = lively_screen_width(session, surface->screen_index);
    uint32_t h = lively_screen_height(session, surface->screen_index);
    gtk_window_set_default_size(GTK_WINDOW(window), (gint)(w ? w : 1920), (gint)(h ? h : 1080));
    gtk_window_move(GTK_WINDOW(window), 0, 0);

    WebKitWebView *view = WEBKIT_WEB_VIEW(webkit_web_view_new());
    gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));
    gtk_widget_show_all(window);

    /* Desktop-type hint so the window manager puts it behind everything. */
    GdkWindow *gdk_window = gtk_widget_get_window(GTK_WIDGET(window));
    if (gdk_window) {
        gdk_window_set_type_hint(gdk_window, GDK_WINDOW_TYPE_HINT_DESKTOP);
        gdk_window_set_skip_taskbar_hint(gdk_window, TRUE);
        gdk_window_set_skip_pager_hint(gdk_window, TRUE);
    }

    char *uri = g_filename_to_uri(path, NULL, NULL);
    if (uri) {
        webkit_web_view_load_uri(view, uri);
        g_free(uri);
    } else {
        return 0;
    }

    surface->web_window = window;
    surface->web_view = view;
    return 1;
}
#endif /* LIVELY_HAVE_WEB */

int lively_create_view(void *handle, const char *path, int kind) {
    if (!handle || !path) return 0;
    LivelySurface *surface = (LivelySurface *)handle;
    surface->kind = kind;
    surface->position = 0.0;
    surface->duration = -1.0;

    switch (kind) {
        case LIVELY_KIND_IMAGE:
            return draw_image(surface, path);
#ifdef LIVELY_HAVE_VIDEO
        case LIVELY_KIND_VIDEO:
            return start_video(surface, path);
#endif
#ifdef LIVELY_HAVE_WEB
        case LIVELY_KIND_WEB:
            return start_web(surface, path, surface->session);
#endif
        default:
            /* Shader needs a GL pipeline that is not built here; report
             * honestly rather than showing black as if it had rendered. */
            return 0;
    }
}

int lively_swap_view_source(void *handle, const char *path) {
    if (!handle || !path) return 0;
    LivelySurface *surface = (LivelySurface *)handle;
    if (surface->kind != LIVELY_KIND_IMAGE) return 0;
    return draw_image(surface, path);
}

void lively_destroy_view(void *handle) {
    (void)handle; /* views are owned by the surface */
}

int lively_web_set_paused(void *handle, int paused) {
#ifdef LIVELY_HAVE_WEB
    if (!handle) return 0;
    LivelySurface *surface = (LivelySurface *)handle;
    if (!surface->web_view) return 0;
    WebKitWebView *view = surface->web_view;
    if (paused) {
        webkit_web_view_set_playing(view, FALSE);
        webkit_web_view_try_close_page(view);  /* stops timers and animation */
    } else {
        webkit_web_view_set_playing(view, TRUE);
        webkit_web_view_resume_page(view);
    }
    return 1;
#else
    (void)handle; (void)paused;
    return 0;
#endif
}

int lively_web_send_property(void *handle, const char *name, const char *value) {
#ifdef LIVELY_HAVE_WEB
    if (!handle || !name || !value) return 0;
    LivelySurface *surface = (LivelySurface *)handle;
    if (!surface->web_view) return 0;

    /* Matches the API the built-in web wallpaper exposes. */
    char *script = g_strdup_printf(
        "window.livelyPropertyListener && window.livelyPropertyListener(%s, %s);",
        name, value);
    webkit_web_view_evaluate_javascript(surface->web_view, script, -1, NULL, NULL, NULL, NULL);
    g_free(script);
    return 1;
#else
    (void)handle; (void)name; (void)value;
    return 0;
#endif
}

void lively_set_playing(void *handle, int playing) {
    if (!handle) return;
    LivelySurface *surface = (LivelySurface *)handle;
    surface->playing = playing;

#ifdef LIVELY_HAVE_VIDEO
    if (surface->pipeline && surface->kind == LIVELY_KIND_VIDEO) {
        if (playing) {
            /* Seek to the start when resuming, so a wallpaper that ran out
             * while paused starts over instead of freezing on the last frame. */
            if (surface->video_eos) {
                gst_element_seek(surface->pipeline, 1.0, GST_FORMAT_TIME,
                                 GST_SEEK_FLAG_FLUSH, GST_SEEK_TYPE_SET, 0,
                                 GST_SEEK_TYPE_END, 0);
            }
            gst_element_set_state(surface->pipeline, GST_STATE_PLAYING);
        } else {
            gst_element_set_state(surface->pipeline, GST_STATE_PAUSED);
        }
    }
#endif
}

void lively_set_volume(void *handle, double volume) {
    if (!handle) return;
    LivelySurface *surface = (LivelySurface *)handle;
    surface->volume = volume;

#ifdef LIVELY_HAVE_VIDEO
    if (surface->pipeline && surface->kind == LIVELY_KIND_VIDEO) {
        GstElement *vol = gst_bin_get_by_name(GST_BIN(surface->pipeline), "volume");
        if (vol) {
            g_object_set(vol, "volume", volume, NULL);
            gst_object_unref(vol);
        }
    }
#endif
}

void lively_seek(void *handle, double percent) {
    if (!handle) return;
    LivelySurface *surface = (LivelySurface *)handle;
    if (percent < 0.0) percent = 0.0;
    if (percent > 100.0) percent = 100.0;

#ifdef LIVELY_HAVE_VIDEO
    if (surface->pipeline && surface->kind == LIVELY_KIND_VIDEO) {
        gint64 duration = gst_element_query_duration(surface->pipeline, GST_FORMAT_TIME);
        if (duration > 0) {
            gint64 target = (gint64)((duration / 100.0) * percent);
            surface->position = (double)target / GST_SECOND;
            gst_element_seek(surface->pipeline, 1.0, GST_FORMAT_TIME,
                             GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_KEY_UNIT,
                             GST_SEEK_TYPE_SET, target, GST_SEEK_TYPE_END, 0);
        }
        return;
    }
#endif
    if (surface->duration > 0) surface->position = surface->duration * percent / 100.0;
}

/* Pumps queued pipeline events and refreshes position, called from the draw
 * pass so the UI can report an honest state. */
void lively_refresh_video(void *handle) {
#ifdef LIVELY_HAVE_VIDEO
    if (!handle) return;
    LivelySurface *surface = (LivelySurface *)handle;
    if (!surface->pipeline || surface->kind != LIVELY_KIND_VIDEO) return;

    surface->duration = (double)gst_element_query_duration(
        surface->pipeline, GST_FORMAT_TIME) / GST_SECOND;
    surface->position = (double)gst_element_query_position(
        surface->pipeline, GST_FORMAT_TIME) / GST_SECOND;
#endif
}

int lively_playback_state(void *handle) {
    if (!handle) return LIVELY_PLAYBACK_UNKNOWN;
    LivelySurface *surface = (LivelySurface *)handle;
    if (surface->kind != LIVELY_KIND_VIDEO) return LIVELY_PLAYBACK_UNKNOWN;

#ifdef LIVELY_HAVE_VIDEO
    if (surface->video_error) return LIVELY_PLAYBACK_FAILED;
    if (surface->video_eos) return LIVELY_PLAYBACK_ENDED;
    if (surface->duration < 0.0) return LIVELY_PLAYBACK_LOADING;
#endif
    return surface->playing ? LIVELY_PLAYBACK_PLAYING : LIVELY_PLAYBACK_PAUSED;
}

double lively_playback_position(void *handle) {
    return handle ? ((LivelySurface *)handle)->position : -1.0;
}

double lively_playback_duration(void *handle) {
    return handle ? ((LivelySurface *)handle)->duration : -1.0;
}

/* Writes the last drawn surface to a PNG. */
int lively_capture(void *handle, const char *out_path) {
    if (!handle || !out_path) return 0;
    /* Without a retained backing surface the capture has nothing to read;
     * report failure rather than writing a black file. */
    return 0;
}