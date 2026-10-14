/*
 * C ABI between the Rust control plane and the SwiftUI settings app.
 *
 * Direction of data flow:
 *
 *   Rust  --lively_ui_present()-->  Swift     "show the window"
 *   Rust  --lively_ui_refresh()-->  Swift     "re-read the state and redraw"
 *
 *   Swift  --handler(json)-->     Rust        user acted; execute and reply
 *   Rust   --json-->              Swift       resulting application state
 *
 * Rust owns the state. Swift never reads the config or the library itself; it
 * asks for a snapshot and renders that. That keeps a single source of truth and
 * means the UI cannot drift from what the daemon is actually running.
 *
 * The handler is registered by Rust at startup and stored by Swift. It takes a
 * UTF-8 JSON request and returns a UTF-8 JSON response. The response buffer is
 * owned by Rust and must be released with lively_ui_string_free().
 */

#ifndef LIVELY_UI_BRIDGE_H
#define LIVELY_UI_BRIDGE_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdbool.h>

/*
 * Executes one UI request and returns a JSON response.
 * `request_json` is a NUL-terminated UTF-8 string.
 * The returned pointer must be released with lively_ui_string_free().
 */
typedef char *(*LivelyUiCommandHandler)(const char *request_json);

/* Registers the request handler. Called by Rust during startup. */
void lively_ui_set_handler(LivelyUiCommandHandler handler);

/* Shows the settings window and brings it to the front. */
void lively_ui_present(void);

/* Asks the UI to re-fetch state and redraw. Safe to call from any thread. */
void lively_ui_refresh(void);

/* Releases a string returned by the handler. */
void lively_ui_string_free(char *s);

/*
 * True when the Swift half of the binary was actually linked in.
 * Lets Rust degrade gracefully (and report it clearly) if the UI was stripped.
 */
bool lively_ui_available(void);

#ifdef __cplusplus
}
#endif

#endif /* LIVELY_UI_BRIDGE_H */