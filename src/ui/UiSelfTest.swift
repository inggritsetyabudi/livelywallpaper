import Foundation
import SwiftUI

/// Exercises the real UI code path — Swift view model → Rust handler → back —
/// and logs pass/fail per step.
///
/// Why this exists: the state pipeline can be verified from the Rust side with a
/// probe, and that pipeline can be completely green while the window is broken,
/// because the probe bypasses the Swift half. An earlier bug — the handler
/// never reaching Swift — passed every Rust-side test. Anything that only tests
/// the Rust side is not testing the UI.
///
/// Runs when `LIVELY_UI_SELFTEST=1`. Never invoked in normal use.
enum UiSelfTest {

    private static var failures: [String] = []
    private static var checks = 0

    static func runIfRequested(model: AppModel) {
        guard ProcessInfo.processInfo.environment["LIVELY_UI_SELFTEST"] == "1" else { return }
        // Give the first load a moment so the check reports steady state.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            execute(model: model)
        }
    }

    private static func check(_ label: String, _ condition: Bool, _ detail: @autoclosure () -> String = "") {
        checks += 1
        if condition {
            NSLog("[LivelyUITest] PASS  %@ %@", label, detail())
        } else {
            failures.append(label)
            NSLog("[LivelyUITest] FAIL  %@ %@", label, detail())
        }
    }

    @MainActor
    private static func execute(model: AppModel) {
        NSLog("[LivelyUITest] ===== begin =====")

        // The handler must be installed, or nothing below can work.
        check("handler installed", !(model.errorMessage?.contains("not connected") ?? false),
              model.errorMessage ?? "")

        model.reload()

        // --- library ---------------------------------------------------------
        check("library is not empty", !model.isLibraryEmpty, "(\(model.library.count) items)")
        check("library loads built-ins", model.builtins.count >= 6, "(\(model.builtins.count))")
        check("built-ins partition correctly",
              model.builtins.allSatisfy(\.isBuiltin) && model.imported.allSatisfy { !$0.isBuiltin },
              "(builtin \(model.builtins.count) / imported \(model.imported.count))")

        // --- search ----------------------------------------------------------
        if let first = model.builtins.first {
            model.searchText = first.title
            check("search narrows results", model.filteredLibrary.count < model.library.count,
                  "(\(model.filteredLibrary.count) of \(model.library.count))")
            check("search keeps the match", model.filteredLibrary.contains { $0.id == first.id })

            model.searchText = "zzz-no-such-wallpaper-zzz"
            check("search with no match is empty", model.filteredLibrary.isEmpty)
            model.searchText = ""
            check("clearing search restores all", model.filteredLibrary.count == model.library.count)
        }

        // --- selection and metadata ------------------------------------------
        if let target = model.builtins.first {
            model.select(target)
            check("selection takes effect", model.selectedId == target.id)

            let shown = model.library.first { $0.id == model.selectedId }
            check("selected wallpaper is in the model", shown != nil)
            check("metadata has a title", !(shown?.title.isEmpty ?? true))
            check("metadata has an author", !(shown?.author.isEmpty ?? true))
            check("thumbnail path resolves", !(shown?.thumbnailPath ?? "").isEmpty)
        }

        // --- apply / close ---------------------------------------------------
        if let target = model.builtins.first, let screen = model.screens.first {
            model.targetScreen = screen.index
            model.select(target)

            model.applySelected()
            let applied = model.screens.first { $0.index == screen.index }
            check("apply reports success", model.errorMessage == nil, model.errorMessage ?? "")
            check("apply lands on the target screen", applied?.currentId == target.id,
                  "(current: \(applied?.currentId ?? "nil"))")
            check("apply sets a status message", model.statusMessage != nil)

            model.clearScreen(screen.index)
            let cleared = model.screens.first { $0.index == screen.index }
            check("clear empties the screen", cleared?.currentId == nil)
        }

        // --- transport --------------------------------------------------------
        let beforeVolume = model.config?.volumePercent ?? 0
        model.setVolume(55)
        check("volume changes", model.config?.volumePercent == 55,
              "(\(beforeVolume) -> \(model.config.map { String($0.volumePercent) } ?? "unset"))")

        model.setVolume(Double(model.config?.volumePercent ?? 55))
        let wasMuted = model.config?.isMuted ?? false
        model.toggleMute()
        check("mute toggles", (model.config?.isMuted ?? wasMuted) != wasMuted)

        let wasPaused = model.config?.isPaused ?? false
        model.setPlaying(true)
        check("play sets playing", model.config?.isPaused == false)
        model.setPlaying(false)
        check("pause sets paused", model.config?.isPaused == true)
        model.setPlaying(true)
        check("resume clears paused", model.config?.isPaused == false)

        // --- settings ---------------------------------------------------------
        model.setConfig("pauseOnBattery", !model.config!.pauseOnBattery)
        check("config toggle applies", model.config?.pauseOnBattery != nil)

        // Out-of-range must be refused, not silently clamped.
        model.setConfig("layoutMode", 99)
        check("invalid layout is refused",
              model.errorMessage?.contains("layout mode") ?? false,
              model.errorMessage ?? "(no error reported)")

        // --- errors surface ---------------------------------------------------
        model.send("definitelyNotAnAction")
        check("unknown action surfaces an error", model.errorMessage != nil,
              model.errorMessage ?? "")

        model.select(model.builtins[0])
        model.send("apply", ["id": "no-such-id", "screen": 0])
        check("bad id surfaces an error", model.errorMessage != nil,
              model.errorMessage ?? "")
        model.dismissError()

        // --- properties -------------------------------------------------------
        let withProps = model.builtins.first { $0.wallpaperType == 6 }
            ?? model.builtins.first { $0.wallpaperType == 1 }
        if let withProps {
            model.select(withProps)
            check("wallpaper properties load", !model.properties.isEmpty,
                  "(\(model.properties.count) controls)")
            let sliders = model.properties.filter { $0.propType == "slider" }
            check("slider properties carry a range",
                  sliders.allSatisfy { $0.min != nil && $0.max != nil },
                  "(\(sliders.count) sliders)")
        }

        // --- summary ----------------------------------------------------------
        if failures.isEmpty {
            NSLog("[LivelyUITest] ===== ALL \(checks) CHECKS PASSED =====")
        } else {
            NSLog("[LivelyUITest] ===== \(failures.count)/\(checks) FAILED: %@ =====",
                  failures.joined(separator: ", "))
        }
    }
}
