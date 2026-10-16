/* C ABI for the Linux backend. Mirrors src/platform/mod.rs. */
#ifndef LIVELY_LINUX_H
#define LIVELY_LINUX_H

#include <stdbool.h>
#include <stdint.h>

#define LIVELY_SESSION_NONE 0
#define LIVELY_SESSION_WAYLAND 1
#define LIVELY_SESSION_X11 2

#define LIVELY_KIND_VIDEO 0
#define LIVELY_KIND_WEB 1
#define LIVELY_KIND_SHADER 2
#define LIVELY_KIND_IMAGE 3

uint32_t lively_screen_count(int session);
uint32_t lively_screen_width(int session, uint32_t index);
uint32_t lively_screen_height(int session, uint32_t index);

/* Display fd for poll(2), or -1 when headless. Lets the daemon block instead
 * of spinning; see lively_event_fd in the .c for why this matters. */
int lively_event_fd(void);

/* True when some window is fullscreen. Returns false when it cannot tell --
 * an unanswered question must not be reported as "yes", or wallpapers would
 * pause for reasons the user cannot see. */
bool lively_x11_any_fullscreen(void);

void *lively_create_surface(int session, uint32_t screen_index);
void lively_destroy_surface(void *handle);

int lively_create_view(void *handle, const char *path, int kind);
int lively_swap_view_source(void *handle, const char *path);
void lively_destroy_view(void *handle);
int lively_web_set_paused(void *handle, int paused);
int lively_web_send_property(void *handle, const char *name, const char *value);
void lively_set_playing(void *handle, int playing);
void lively_refresh_video(void *handle);
void lively_set_volume(void *handle, double volume);
void lively_seek(void *handle, double percent);
int lively_playback_state(void *handle);
double lively_playback_position(void *handle);
double lively_playback_duration(void *handle);
int lively_capture(void *handle, const char *out_path);

#endif /* LIVELY_LINUX_H */
