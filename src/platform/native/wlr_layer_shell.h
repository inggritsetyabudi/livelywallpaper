/*
 * Hand-written bindings for zwlr_layer_shell_v1.
 *
 * The usual way to get these is `wayland-scanner`, which turns the protocol
 * XML into C at build time. That is not available on a macOS build host --
 * it is a Linux binary -- so this file spells the two interfaces out
 * directly against libwayland's marshalling API.
 *
 * Only the requests and events this backend actually uses are declared. The
 * opcodes below are positional: they are fixed by the protocol XML and must
 * match the order of the requests in wlr-layer-shell-unstable-v1.xml. Adding a
 * new request to the XML does not renumber the existing ones, but *reordering*
 * them would silently corrupt every call here.
 *
 *   zwlr_layer_shell_v1        0 get_layer_surface   1 destroy
 *   zwlr_layer_surface_v1       0 set_size            1 set_anchor
 *                               2 set_exclusive_zone  3 set_margin
 *                               4 set_keyboard_interactivity
 *                               5 get_popup           6 destroy
 *                               7 ack_configure       8 set_layer   (v4)
 *
 * Events:
 *   zwlr_layer_shell_v1        0 error
 *   zwlr_layer_surface_v1      0 configure           1 closed
 */

#ifndef LIVELY_WLR_LAYER_SHELL_H
#define LIVELY_WLR_LAYER_SHELL_H

#include <stdint.h>
#include <wayland-client.h>

#ifdef __cplusplus
extern "C" {
#endif

struct zwlr_layer_shell_v1;
struct zwlr_layer_surface_v1;

/* layer values, from the protocol's `<enum name="layer">`. */
#define ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND 0
#define ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM 1
#define ZWLR_LAYER_SHELL_V1_LAYER_TOP 2
#define ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY 3

/* anchor bits, from the protocol's `<enum name="anchor">`. */
#define ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP (1 << 0)
#define ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM (1 << 1)
#define ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT (1 << 2)
#define ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT (1 << 3)

/* Defined at the bottom of this file. Declared here so the inline wrappers can
 * reference them; the bodies come after the `wl_interface` type is complete. */
static const struct wl_interface zwlr_layer_shell_v1_interface;
static const struct wl_interface zwlr_layer_surface_v1_interface;

struct zwlr_layer_shell_v1_listener {
    void (*error)(void *data, struct zwlr_layer_shell_v1 *shell, uint32_t code,
                  const char *message, void *id);
};

struct zwlr_layer_surface_v1_listener {
    /* Sent when the compositor assigns the surface a size. `serial` must be
     * handed back with ack_configure or the surface stays unconfigured. */
    void (*configure)(void *data, struct zwlr_layer_surface_v1 *surface,
                      uint32_t serial, uint32_t width, uint32_t height);
    void (*closed)(void *data, struct zwlr_layer_surface_v1 *surface);
};

static inline struct zwlr_layer_shell_v1_listener *
zwlr_layer_shell_v1_listener_init(void (*error)(void *, struct zwlr_layer_shell_v1 *,
                                               uint32_t, const char *, void *)) {
    static struct zwlr_layer_shell_v1_listener l;
    l.error = error;
    return &l;
}

static inline struct zwlr_layer_surface_v1_listener *
zwlr_layer_surface_v1_listener_init(void (*configure)(void *, struct zwlr_layer_surface_v1 *,
                                                     uint32_t, uint32_t, uint32_t),
                                    void (*closed)(void *, struct zwlr_layer_surface_v1 *)) {
    static struct zwlr_layer_surface_v1_listener l;
    l.configure = configure;
    l.closed = closed;
    return &l;
}

/* Creates a layer surface from the shell. `namespace` is the XDG-style
 * identifier the compositor uses for its output, e.g. "lively-wallpaper". */
static inline struct zwlr_layer_surface_v1 *
zwlr_layer_shell_v1_get_layer_surface(struct zwlr_layer_shell_v1 *shell,
                                      struct wl_surface *surface,
                                      struct wl_output *output,
                                      uint32_t layer,
                                      const char *namespace_) {
    /* marshal_constructor, not marshal: this request creates a new object, so
     * the connection allocates an id for it and returns the new proxy.
     * wl_proxy_marshal returns void and would lose it. */
    return (struct zwlr_layer_surface_v1 *)wl_proxy_marshal_constructor(
        (struct wl_proxy *)shell, 0, &zwlr_layer_shell_v1_interface,
        zwlr_layer_shell_v1_interface.version, surface, output, layer, namespace_);
}

static inline void
zwlr_layer_surface_v1_set_size(struct zwlr_layer_surface_v1 *surface, uint32_t width,
                               uint32_t height) {
    wl_proxy_marshal((struct wl_proxy *)surface, 0, NULL, width, height);
}

static inline void
zwlr_layer_surface_v1_set_anchor(struct zwlr_layer_surface_v1 *surface, uint32_t anchor) {
    wl_proxy_marshal((struct wl_proxy *)surface, 1, NULL, anchor);
}

static inline void
zwlr_layer_surface_v1_set_exclusive_zone(struct zwlr_layer_surface_v1 *surface, int32_t zone) {
    wl_proxy_marshal((struct wl_proxy *)surface, 2, NULL, zone);
}

static inline void
zwlr_layer_surface_v1_set_keyboard_interactivity(struct zwlr_layer_surface_v1 *surface,
                                                  uint32_t interactivity) {
    wl_proxy_marshal((struct wl_proxy *)surface, 4, NULL, interactivity);
}

static inline void zwlr_layer_shell_v1_destroy(struct zwlr_layer_shell_v1 *shell) {
    wl_proxy_destroy((struct wl_proxy *)shell);
}

static inline void zwlr_layer_surface_v1_destroy(struct zwlr_layer_surface_v1 *surface) {
    wl_proxy_destroy((struct wl_proxy *)surface);
}

static inline void zwlr_layer_surface_v1_ack_configure(struct zwlr_layer_surface_v1 *surface,
                                                       uint32_t serial) {
    wl_proxy_marshal((struct wl_proxy *)surface, 7, NULL, serial);
}

static inline void
zwlr_layer_surface_v1_add_listener(struct zwlr_layer_surface_v1 *surface,
                                   struct zwlr_layer_surface_v1_listener *listener,
                                   void *data) {
    wl_proxy_add_listener((struct wl_proxy *)surface, (void (**)(void))listener, data);
}

static inline void
zwlr_layer_shell_v1_add_listener(struct zwlr_layer_shell_v1 *shell,
                                 struct zwlr_layer_shell_v1_listener *listener, void *data) {
    wl_proxy_add_listener((struct wl_proxy *)shell, (void (**)(void))listener, data);
}

/*
 * The interface structs and their type tables.
 *
 * `types` lists the argument types in declaration order, including the object
 * arguments: libwayland needs this to know how many bytes to marshal for each.
 */
/*
 * struct wl_interface has six fields:
 *   name, version, method_count, methods, event_count, events
 *
 * All six are written out. Relying on partial initialisation here silently
 * shifts `methods` into `event_count` and produces a struct that looks fine
 * and marshals garbage.
 *
 * Event signatures for reference (the events themselves are dispatched through
 * the listener, but the counts must still be right for the wire format):
 *   zwlr_layer_shell_v1.error  uus  -> code, message, id
 *   zwlr_layer_surface_v1.configure  uii -> serial, width, height
 *   zwlr_layer_surface_v1.closed     ""  -> (none)
 */
static const struct wl_message zwlr_layer_shell_v1_requests[] = {
    {"get_layer_surface", "uuuus", 0},
    {"destroy", "", 0},
};

static const struct wl_message zwlr_layer_shell_v1_events[] = {
    {"error", "uus", 0},
};

static const struct wl_message zwlr_layer_surface_v1_requests[] = {
    {"set_size", "ii", 0},
    {"set_anchor", "u", 0},
    {"set_exclusive_zone", "i", 0},
    {"set_margin", "iiii", 0},
    {"set_keyboard_interactivity", "u", 0},
    {"get_popup", "n?oo", 0},
    {"destroy", "", 0},
    {"ack_configure", "u", 0},
    {"set_layer", "u", 0},
};

static const struct wl_message zwlr_layer_surface_v1_events[] = {
    {"configure", "uii", 0},
    {"closed", "", 0},
};

static const struct wl_interface zwlr_layer_shell_v1_interface = {
    "zwlr_layer_shell_v1", 4,
    2, zwlr_layer_shell_v1_requests,
    1, zwlr_layer_shell_v1_events,
};

static const struct wl_interface zwlr_layer_surface_v1_interface = {
    "zwlr_layer_surface_v1", 4,
    9, zwlr_layer_surface_v1_requests,
    2, zwlr_layer_surface_v1_events,
};

#ifdef __cplusplus
}
#endif

#endif /* LIVELY_WLR_LAYER_SHELL_H */