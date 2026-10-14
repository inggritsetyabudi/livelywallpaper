#pragma once
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Wallpaper Type Enum matching Lively Wallpaper
typedef enum {
    WALLPAPER_TYPE_VIDEO = 0,
    WALLPAPER_TYPE_WEB = 1,
    WALLPAPER_TYPE_WEB_GL = 2,
    WALLPAPER_TYPE_APP = 3,
    WALLPAPER_TYPE_GIF = 4,
    WALLPAPER_TYPE_PICTURE = 5,
    WALLPAPER_TYPE_SHADER = 6
} LivelyWallpaperType;

// Scaling Mode
typedef enum {
    SCALE_ASPECT_FILL = 0,
    SCALE_ASPECT_FIT = 1,
    SCALE_STRETCH = 2,
    SCALE_CENTER = 3
} LivelyScaleMode;

// Display arrangement
typedef enum {
    LAYOUT_PER_SCREEN = 0,
    LAYOUT_SPAN = 1,
    LAYOUT_DUPLICATE = 2
} LivelyLayoutMode;

// Init & Lifecycle
bool lively_init_app(void);
void lively_run_loop_run(void);
void lively_run_loop_stop(void);
void lively_pump_events(double timeout_secs);

// Thread affinity probe. True when called on the process's main thread.
// AppKit-touching functions below are only safe to call when this returns true.
bool lively_is_main_thread(void);

// Screen / Monitor info
uint32_t lively_get_screen_count(void);
bool lively_get_screen_bounds(uint32_t index, double *x, double *y, double *w, double *h);
// Screen name as a newly allocated C string.
// The caller OWNS the result and must release it with lively_free_string();
const char* lively_get_screen_name(uint32_t index);

// Releases a string returned by this library (currently lively_get_screen_name).
void lively_free_string(char* s);

// Desktop Window Management
void* lively_create_desktop_window(uint32_t screen_index);
void lively_destroy_desktop_window(void* win_ptr);
void lively_set_window_level(void* win_ptr, int32_t level_offset);
void lively_set_window_interactive(void* win_ptr, bool interactive);

// Video Engine (AVFoundation)
void* lively_create_video_view(void* win_ptr, const char* file_path, LivelyScaleMode scale_mode);
// Point an existing video view at a different file. Returns 0 on failure.
int lively_video_set_source(void* player_ptr, const char* file_path);
void lively_video_set_volume(void* player_ptr, float volume);
void lively_video_set_playback_rate(void* player_ptr, float rate);
void lively_video_seek(void* player_ptr, double percent);
void lively_video_play(void* player_ptr);
// Playback state: 0 unknown, 1 loading, 2 ready, 3 playing, 4 paused,
// 5 failed, 6 ended. Lets the UI distinguish "still loading" from "broken".
int32_t lively_video_get_state(void* player_ptr);
double lively_video_get_position(void* player_ptr);
double lively_video_get_duration(void* player_ptr);
void lively_video_pause(void* player_ptr);
void lively_destroy_video_view(void* player_ptr);

// Web Engine (WKWebView)
void* lively_create_web_view(void* win_ptr, const char* url_or_path, bool is_local_file);
void lively_web_eval_js(void* web_ptr, const char* js_code);
void lively_web_send_audio_spectrum(void* web_ptr, const float* bands, uint32_t count);
void lively_web_send_system_data(void* web_ptr, const char* json_data);
void lively_web_send_property(void* web_ptr, const char* prop_name, const char* prop_value);
void lively_web_reload(void* web_ptr);
void lively_web_pause(void* web_ptr);
void lively_web_resume(void* web_ptr);
void lively_destroy_web_view(void* web_ptr);

// Metal Shader Engine (CAMetalLayer)
void* lively_create_shader_view(void* win_ptr, const char* shader_source);
void lively_shader_set_pause(void* shader_ptr, bool paused);
void lively_shader_update_audio(void* shader_ptr, const float* bands, uint32_t count);
void lively_destroy_shader_view(void* shader_ptr);

// GIF / Picture Engine (NSImageView)
void* lively_create_image_view(void* win_ptr, const char* file_path, LivelyScaleMode scale_mode);
// Point an existing image view at a different file. Returns 0 on failure.
int lively_image_set_source(void* img_ptr, const char* file_path);
void lively_destroy_image_view(void* img_ptr);

// Performance / Automation checks
bool lively_is_any_app_fullscreen(void);
bool lively_is_running_on_battery(void);

// Menu Bar Tray & Native UI
typedef void (*TrayCallback)(int command_id);
void lively_setup_tray_menu(TrayCallback callback);
void lively_update_tray_title(const char* title);
void lively_open_file_picker(char* out_path, uint32_t max_len);

// Main Dispatch. Runs `fn(ctx)` on the main thread: immediately if the caller is
// already on it, otherwise asynchronously on the main queue.
// `fn` must be a plain C function pointer; `ctx` is passed through untouched.
void lively_dispatch_on_main(void (*fn)(void*), void* ctx);

// Screenshot
bool lively_take_screenshot(void* win_ptr, const char* out_path);

// Screen Saver Mode
void* lively_create_screensaver_window(uint32_t screen_index);
void lively_close_screensaver_window(void* win_ptr);
void lively_start_screensaver_monitor(void (*dismiss_cb)(void));
void lively_stop_screensaver_monitor(void);

#ifdef __cplusplus
}
#endif
