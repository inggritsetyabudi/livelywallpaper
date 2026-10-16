import AppKit
import Foundation
import os
import SwiftUI

// MARK: - C bridge

/// Function pointer Rust registers at startup.
public typealias UiCommandHandler = @convention(c) (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?

private var commandHandler: UiCommandHandler?

/// Minimal logger: NSLog is visible in Console.app and in the unified log,
/// which matters more here than pretty output.
enum log {
    static func info(_ message: String) {
        NSLog("[LivelyUI] %@", message)
    }
}

/// Stores the handler Rust handed us.
///
/// Named differently from the Rust-side `lively_ui_set_handler` on purpose: two
/// symbols with the same name meant Rust called its own copy, the Swift side
/// stayed nil, and every UI action failed with "not connected to the daemon".
/// Rust is the only caller.
@_cdecl("lively_ui_install_handler")
public func lively_ui_install_handler(_ raw: UInt) {
    guard raw != 0 else {
        commandHandler = nil
        return
    }
    commandHandler = unsafeBitCast(raw, to: UiCommandHandler.self)
}

/// Releases a string returned by the Rust handler.
@_cdecl("lively_ui_string_free")
public func lively_ui_string_free(_ s: UnsafeMutablePointer<CChar>?) {
    guard let s else { return }
    free(s)
}

/// True when the Swift half is present. Always true in this binary; Rust uses it
/// to report clearly if the UI was ever stripped out at build time.
@_cdecl("lively_ui_available")
public func lively_ui_available() -> Bool { true }

/// Asks the UI to re-fetch state and redraw.
@_cdecl("lively_ui_refresh")
public func lively_ui_refresh() {
    DispatchQueue.main.async {
        AppModel.shared.reload()
    }
}

/// Shows the settings window.
@_cdecl("lively_ui_present")
public func lively_ui_present() {
    DispatchQueue.main.async {
        AppModel.shared.presentWindow()
    }
}

// MARK: - Model

/// Everything the settings window renders, mirroring what the daemon reports.
struct LibraryEntry: Identifiable, Decodable, Hashable {
    let id: String
    let title: String
    let author: String
    /// Ships with the app rather than imported by the user.
    let isBuiltin: Bool
    let license: String
    let wallpaperType: Int
    let typeLabel: String
    let hasThumbnail: Bool
    let thumbnailPath: String?
    let description: String
}

struct ScreenEntry: Identifiable, Decodable, Hashable {
    let index: UInt32
    let name: String
    let width: UInt32
    let height: UInt32
    let currentTitle: String?
    let currentId: String?

    var id: UInt32 { index }
}

/// Which sidebar page the detail column is showing.
///
/// Named `SidebarPage` rather than `Section`: SwiftUI has its own `Section`,
/// and a same-named type here silently shadowed it inside the sidebar.
enum SidebarPage: Hashable {
    case library
    case screens
    case settings
}

struct ConfigView: Decodable, Hashable {
    let volume: Float
    let volumePercent: UInt32
    let isMuted: Bool
    let isPaused: Bool
    let pauseOnFullscreen: Bool
    let pauseOnBattery: Bool
    let layoutMode: Int
    let audioVisualizerEnabled: Bool
}

struct PropertyEntry: Identifiable, Decodable, Hashable {
    let key: String
    let propType: String
    let label: String
    let value: JSONValue?
    let min: Double?
    let max: Double?
    let step: Double?
    let options: [String]

    var id: String { key }
}

struct StateSnapshot: Decodable {
    let library: [LibraryEntry]
    let screens: [ScreenEntry]
    let config: ConfigView
    let properties: [PropertyEntry]
}

struct UiResponse: Decodable {
    let ok: Bool
    let error: String?
    /// Absent for actions that do not change app state, so a playback poll does
    /// not carry a whole library snapshot the caller would discard.
    let state: StateSnapshot?
    /// Present only for `playbackStatus`; null means "no video on that screen".
    let playback: PlaybackView??
}

struct PlaybackView: Decodable {
    /// 0 unknown, 1 loading, 2 ready, 3 playing, 4 paused, 5 failed, 6 ended.
    let state: Int
    /// Human-readable state, or null when it is simply playing.
    let label: String?
    let position: Double
    let duration: Double
}

/// `Codable` cannot round-trip a heterogeneous JSON scalar, and property values
/// really are heterogeneous (numbers, booleans, colour strings), so wrap the
/// raw `Any` JSON fragment instead.
enum JSONValue: Decodable, Hashable {
    case number(Double)
    case bool(Bool)
    case string(String)
    case null

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let v = try? container.decode(Bool.self) {
            self = .bool(v)
        } else if let v = try? container.decode(Double.self) {
            self = .number(v)
        } else {
            self = .string(try container.decode(String.self))
        }
    }

    var stringValue: String {
        switch self {
        case .string(let s): return s
        case .number(let n): return n == n.rounded() ? String(Int(n)) : String(n)
        case .bool(let b): return b ? "true" : "false"
        case .null: return ""
        }
    }

    var doubleValue: Double {
        switch self {
        case .number(let n): return n
        case .bool(let b): return b ? 1 : 0
        case .string(let s): return Double(s) ?? 0
        case .null: return 0
        }
    }

    var boolValue: Bool {
        switch self {
        case .bool(let b): return b
        case .number(let n): return n != 0
        case .string(let s): return s == "true" || s == "1"
        case .null: return false
        }
    }
}

// MARK: - View model

/// Single source of truth for the window.
///
/// Holds no business logic: every mutation is a command to Rust followed by
/// whatever state Rust reports back. That keeps the UI from drifting out of sync
/// with the daemon — the failure mode where a control shows "on" but the
/// wallpaper is paused.
@MainActor
final class AppModel: NSObject, ObservableObject {
    static let shared = AppModel()

    @Published var library: [LibraryEntry] = []
    @Published var screens: [ScreenEntry] = []
    @Published var config: ConfigView?
    @Published var properties: [PropertyEntry] = []

    /// Which sidebar page is showing.
    ///
    /// The sidebar used `NavigationLink(value:)` with no `NavigationStack` and
    /// no `navigationDestination`, so clicking "Screens" or "Settings" did
    /// nothing at all: no destination existed to push to. Selection state has
    /// to live somewhere real for those pages to render.
    @Published var section: SidebarPage = .library

    @Published var selectedId: String?
    @Published var targetScreen: UInt32 = 0
    @Published var searchText: String = ""
    @Published var statusMessage: String?
    @Published var errorMessage: String?
    @Published var isBusy = false

    /// State of the video on the targeted screen, when there is one. Lets the
    /// window say "Loading…" or "Failed to load" instead of showing black.
    @Published var playback: PlaybackView?

    /// Title of the current selection, for status messages.
    var selectedTitle: String {
        library.first { $0.id == selectedId }?.title ?? "wallpaper"
    }

    private var window: NSWindow?

    /// Captured from the most recent reply, since `send` returns only success.
    private var lastPlayback: PlaybackView?

    // MARK: Window

    override init() {
        super.init()
    }

    func presentWindow() {
        // The daemon runs as an accessory (menu-bar only, no Dock icon). To show
        // a window that can actually take focus we have to become a regular app;
        // switch back to accessory when the window closes so the Dock icon and
        // the menu bar app do not linger.
        if window == nil {
            NSApp.setActivationPolicy(.regular)
        }

        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }

        let hosting = NSHostingController(rootView: ContentView().environmentObject(self))
        let window = NSWindow(contentViewController: hosting)
        window.title = "Lively Wallpaper"
        window.setContentSize(NSSize(width: 1040, height: 660))
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
        window.center()
        window.isReleasedWhenClosed = false
        window.setFrameAutosaveName("LivelyMainWindow")

        // Closing the settings window returns the app to being menu-bar only.
        window.delegate = self

        self.window = window
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

reload()

        // Only does anything when LIVELY_UI_SELFTEST=1.
        UiSelfTest.runIfRequested(model: self)
    }

    // MARK: Derived

    var filteredLibrary: [LibraryEntry] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return library }
        return library.filter {
            $0.title.localizedCaseInsensitiveContains(query)
                || $0.author.localizedCaseInsensitiveContains(query)
        }
    }

    var isLibraryEmpty: Bool { library.isEmpty }
    var hasSelection: Bool { selectedId != nil }

    /// Wallpapers that ship with the app, shown first.
    var builtins: [LibraryEntry] { filteredLibrary.filter(\.isBuiltin) }

    /// Wallpapers the user imported.
    var imported: [LibraryEntry] { filteredLibrary.filter { !$0.isBuiltin } }

    // MARK: Commands

    /// Send a command to Rust and adopt the state it returns.
    ///
    /// `refreshSelection` carries the currently selected id so the response
    /// includes that wallpaper's properties.
    @discardableResult
    func send(_ action: String, _ fields: [String: Any] = [:]) -> Bool {
        guard let handler = commandHandler else {
            errorMessage = "The settings UI is not connected to the daemon."
            return false
        }

        var payload = fields
        payload["action"] = action
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else {
            errorMessage = "Could not encode the \(action) request."
            return false
        }

        isBusy = true
        defer { isBusy = false }

        // The handler blocks while Rust mutates the engine. It is fast (sub-
        // millisecond), and keeping it synchronous is what guarantees the state
        // we render is the state after the action — no window where the controls
        // and the daemon disagree.
        let response: UiResponse? = json.withCString { cString -> UiResponse? in
            guard let raw = handler(cString) else {
                errorMessage = "The daemon did not answer the \(action) request."
                return nil
            }
            defer { lively_ui_string_free(raw) }
            let text = String(cString: raw)
            return try? JSONDecoder().decode(UiResponse.self, from: Data(text.utf8))
        }

        guard let response else {
            if errorMessage == nil {
                errorMessage = "The daemon sent a reply we could not read."
            }
            return false
        }

        apply(response)
        return response.ok
    }

    private func apply(_ response: UiResponse) {
        if let state = response.state {
            library = state.library
            screens = state.screens
            config = state.config
            properties = state.properties

            // Keep the selection valid as items come and go.
            if let id = selectedId, !library.contains(where: { $0.id == id }) {
                selectedId = nil
            }
            if selectedId == nil {
                selectedId = library.first?.id
            }
            if !screens.contains(where: { $0.index == targetScreen }) {
                targetScreen = screens.first?.index ?? 0
            }
        }

        if let playback = response.playback ?? nil {
            self.playback = playback
            lastPlayback = playback
        }

        if response.ok {
            errorMessage = nil
        } else {
            errorMessage = response.error ?? "That action did not work."
        }
    }

    func reload() {
        guard send("state", ["id": selectedId ?? NSNull()]) else {
            // Logged rather than only shown in the alert: if the window opens
            // blank this is the one line that says why.
            NSLog("[LivelyUI] could not load state: %@", errorMessage ?? "unknown")
            return
        }
        log.info("library: \(library.count) items (\(builtins.count) built-in)")
    }

    func select(_ entry: LibraryEntry) {
        selectedId = entry.id
        _ = send("select", ["id": entry.id])
    }

    func applySelected() {
        guard let id = selectedId else { return }
        let ok = send("apply", ["id": id, "screen": Int(targetScreen)])
        if ok {
            let name = library.first { $0.id == id }?.title ?? "Wallpaper"
            statusMessage = "Applied “\(name)” to screen \(targetScreen + 1)."
        }
    }

    func closeWallpaper(on screen: UInt32) {
        _ = send("closeWallpaper", ["monitor": Int(screen)])
    }

    func importWallpaper(at path: String) {
        if send("addLibrary", ["path": path]) {
            statusMessage = "Imported “\(URL(fileURLWithPath: path).lastPathComponent)”."
        }
    }

    func remove(_ entry: LibraryEntry) {
        if send("removeLibrary", ["id": entry.id]) {
            statusMessage = "Removed “\(entry.title)” from the library."
        }
    }

    /// Ask the daemon how the video on `screen` is doing.
    func refreshPlayback(screen: UInt32) {
        if send("playbackStatus", ["screen": Int(screen)]) {
            playback = lastPlayback
        }
    }

    func setVolume(_ percent: Double) {
        _ = send("volume", ["value": percent])
    }

    func toggleMute() {
        _ = send("toggleMute")
    }

    func setPlaying(_ playing: Bool) {
        _ = send("setPlaying", ["value": playing])
    }

    func setConfig(_ key: String, _ value: Any) {
        _ = send("setConfig", ["key": key, "value": value])
    }

    func setProperty(_ key: String, _ value: Any) {
        guard let id = selectedId else { return }
        _ = send("setProperty", ["key": key, "value": value, "id": id, "screen": Int(targetScreen)])
    }

    func dismissError() {
        errorMessage = nil
    }

    /// Remove the wallpaper from a screen, used by the sidebar context menu.
    /// Applies the current selection to a specific screen.
    func applySelected(to index: UInt32) {
        guard let id = selectedId else { return }
        guard send("apply", ["id": id, "screen": Int(index)]) else { return }
        statusMessage = "Applied \(selectedTitle) to screen \(index + 1)."
    }

    /// Applies the current selection to every screen at once.
    ///
    /// Looped client-side rather than adding an `applyAll` IPC action: the
    /// daemon already handles `apply` for one screen, and a second verb would
    /// need its own error paths and tests for no gain.
    func applySelectedToAll() {
        guard selectedId != nil else { return }
        let count = screens.count
        for screen in screens {
            _ = send("apply", ["id": selectedId ?? "", "screen": Int(screen.index)])
        }
        statusMessage = "Applied \(selectedTitle) to all \(count) screen(s)."
    }

    /// Loads the library page, or whichever screen the user was last on.
    func go(_ target: SidebarPage) {
        section = target
    }

    func clearScreen(_ index: UInt32) {
        guard send("closeWallpaper", ["monitor": Int(index)]) else { return }
        statusMessage = "Removed the wallpaper from screen \(index + 1)."
    }
}

// MARK: - Window lifecycle

extension AppModel: NSWindowDelegate {
    /// Back to a menu-bar-only app once the settings window is closed.
    func windowWillClose(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
    }
}
