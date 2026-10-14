import SwiftUI

// MARK: - Root

struct ContentView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        NavigationSplitView {
            sidebar
                .navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 260)
        } detail: {
            HSplitView {
                libraryColumn
                    .frame(minWidth: 300, idealWidth: 380)

                rightColumn
                    .frame(minWidth: 280, idealWidth: 320)
            }
            .frame(minWidth: 640, idealWidth: 700)
        }
        .toolbar { toolbarContent }
        .searchable(text: $model.searchText, placement: .toolbar, prompt: "Search wallpapers")
        .alert(
            "Could not complete that",
            isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.dismissError() } }
            )
        ) {
            Button("OK", role: .cancel) { model.dismissError() }
        } message: {
            Text(model.errorMessage ?? "")
        }
        .overlay(alignment: .bottom) {
            if let message = model.statusMessage, model.errorMessage == nil {
                StatusBar(text: message)
                    .padding(.bottom, 14)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.18), value: model.statusMessage)
        .task { model.reload() }
        // Poll while the selected wallpaper is a video: its state changes
        // asynchronously after mount, so a one-shot read would always be stale.
        .task(id: model.selectedId) {
            guard let entry = model.selectedId.flatMap({ id in
                model.library.first { $0.id == id }
            }), entry.wallpaperType == 0 else {
                model.playback = nil
                return
            }
            while !Task.isCancelled {
                model.refreshPlayback(screen: model.targetScreen)
                try? await Task.sleep(for: .milliseconds(700))
            }
        }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: sidebarSelection) {
            Section("Library") {
                NavigationLink(value: Selection.library) {
                    Label("All Wallpapers", systemImage: "square.grid.2x2")
                }
                NavigationLink(value: Selection.screens) {
                    Label("Screens", systemImage: "display")
                }
                NavigationLink(value: Selection.settings) {
                    Label("Settings", systemImage: "gearshape")
                }
            }

            if !model.screens.isEmpty {
                Section("Apply To") {
                    ForEach(model.screens) { screen in
                        Button {
                            model.targetScreen = screen.index
                            if let id = model.selectedId,
                               let entry = model.library.first(where: { $0.id == id }) {
                                model.select(entry)
                            }
                        } label: {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(screen.name).lineLimit(1)
                                Text(subtitle(for: screen))
                                    .font(.caption)
                                    .foregroundStyle(subtitleTint(for: screen))
                                    .lineLimit(1)
                            }
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button("Remove Wallpaper") {
                                model.closeWallpaper(on: screen.index)
                            }
                            .disabled(screen.currentTitle == nil)
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }

    private func subtitle(for screen: ScreenEntry) -> String {
        screen.currentTitle ?? "No wallpaper"
    }

    private func subtitleTint(for screen: ScreenEntry) -> Color {
        screen.currentTitle == nil ? .secondary : .accentColor
    }

    private var sidebarSelection: Binding<Selection?> {
        Binding(
            get: { model.selectedId.map { Selection.item($0) } },
            set: { newValue in
                guard case .item(let id) = newValue else { return }
                guard let entry = model.library.first(where: { $0.id == id }) else { return }
                model.select(entry)
            }
        )
    }

    // MARK: Library

    private var libraryColumn: some View {
        Group {
            if model.isLibraryEmpty {
                EmptyLibraryView { model.importWallpaper(at: $0) }
            } else {
                ScrollView {
                    // Built-ins first: they are what a new user should try
                    // before importing anything of their own.
                    if !model.builtins.isEmpty {
                        SectionHeader(
                            title: "Featured",
                            subtitle: "Ships with Lively",
                            systemImage: "sparkles"
                        )
                        wallpaperGrid(model.builtins)
                    }

                    if !model.imported.isEmpty {
                        SectionHeader(
                            title: "Your Wallpapers",
                            subtitle: "\(model.imported.count) item\(model.imported.count == 1 ? "" : "s")",
                            systemImage: "folder"
                        )
                        wallpaperGrid(model.imported)
                    }
                }
                .padding(.vertical, 6)
            }
        }
    }

    /// One grid of wallpaper cards.
    private func wallpaperGrid(_ entries: [LibraryEntry]) -> some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 150, maximum: 220), spacing: 14)],
            spacing: 14
        ) {
            ForEach(entries) { entry in
                WallpaperCard(
                    entry: entry,
                    isSelected: entry.id == model.selectedId,
                    isApplied: model.screens.contains { $0.currentId == entry.id }
                ) {
                    model.select(entry)
                }
                .contextMenu {
                    Button("Apply to Screen \(model.targetScreen + 1)") {
                        model.select(entry)
                        model.applySelected()
                    }
                    Divider()
                    Button("Remove from Library", role: .destructive) {
                        model.remove(entry)
                    }
                    // Built-ins cannot be deleted: they come back on next
                    // launch, so offering the option would only mislead.
                    .disabled(entry.isBuiltin)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 18)
    }

    // MARK: Right column

    @ViewBuilder
    private var rightColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let entry = model.library.first(where: { $0.id == model.selectedId }) {
                DetailsPane(entry: entry)
            } else {
                VStack(spacing: 8) {
                    Image(systemName: "wallpaper")
                        .font(.system(size: 34))
                        .foregroundStyle(.tertiary)
                    Text("Select a wallpaper")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxHeight: .infinity)
    }

    // MARK: Toolbar

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            if let config = model.config {
                PlaybackControls(config: config)
            }
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                presentImportPanel()
            } label: {
                Label("Add Wallpaper", systemImage: "plus")
            }
            .help("Import a wallpaper into the library")
        }
    }
}

enum Selection: Hashable {
    case library
    case screens
    case settings
    case item(String)
}

// MARK: - Sidebar child views

struct PlaybackControls: View {
    @EnvironmentObject var model: AppModel
    let config: ConfigView

    var body: some View {
        HStack(spacing: 14) {
            Button {
                model.setPlaying(config.isPaused)
            } label: {
                Image(systemName: config.isPaused ? "play.fill" : "pause.fill")
                    .frame(width: 18, height: 18)
            }
            .buttonStyle(.borderless)
            .help(config.isPaused ? "Resume" : "Pause")

            Button {
                model.toggleMute()
            } label: {
                Image(systemName: config.isMuted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                    .frame(width: 18, height: 18)
            }
            .buttonStyle(.borderless)
            .help(config.isMuted ? "Unmute" : "Mute")

            HStack(spacing: 6) {
                Slider(
                    value: Binding(
                        get: { Double(config.volumePercent) },
                        // Live feedback while dragging, commit on release.
                        set: { _ in }
                    ),
                    in: 0...100,
                    onEditingChanged: { editing in
                        if !editing { model.setVolume(Double(config.volumePercent)) }
                    }
                )
                .frame(width: 110)
                .disabled(config.isMuted)

                Text("\(config.volumePercent)%")
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(width: 34, alignment: .leading)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(.quaternary.opacity(0.35), in: Capsule())
    }
}

/// Heading above a group of wallpaper cards.
struct SectionHeader: View {
    let title: String
    let subtitle: String
    let systemImage: String

    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: systemImage)
                .font(.caption)
                .foregroundStyle(.tint)
            Text(title)
                .font(.subheadline.weight(.semibold))
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 18)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }
}

struct StatusBar: View {
    let text: String

    var body: some View {
        Label(text, systemImage: "checkmark.circle.fill")
            .font(.callout)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(.regularMaterial, in: Capsule())
            .overlay(Capsule().stroke(.separator, lineWidth: 0.5))
            .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
    }
}

// MARK: - Cards and panes

struct WallpaperCard: View {
    let entry: LibraryEntry
    let isSelected: Bool
    let isApplied: Bool
    let onTap: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            thumbnail
            Text(entry.title)
                .font(.callout)
                .lineLimit(1)
                .truncationMode(.tail)
            HStack(spacing: 5) {
                Text(entry.typeLabel)
                    .font(.caption2)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(Color.secondary.opacity(0.18), in: RoundedRectangle(cornerRadius: 3))
                if isApplied {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.caption2)
                        .foregroundStyle(.tint)
                        .help("Currently applied")
                }
            }
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 9)
                .fill(isSelected ? Color.accentColor.opacity(0.16) : Color.clear)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 9)
                .strokeBorder(isSelected ? Color.accentColor : Color.clear, lineWidth: 2)
        )
        .contentShape(RoundedRectangle(cornerRadius: 9))
        .onTapGesture(perform: onTap)
    }

    private var thumbnail: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 6)
                .fill(
                    LinearGradient(
                        colors: [Color.secondary.opacity(0.22), Color.secondary.opacity(0.07)],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

            if let path = entry.thumbnailPath,
               let image = NSImage(contentsOfFile: path) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: symbol)
                    .font(.system(size: 26))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(height: 88)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(.separator, lineWidth: 0.5)
        )
    }

    private var symbol: String {
        switch entry.wallpaperType {
        case 0: return "film"
        case 1, 2: return "globe"
        case 3: return "app.badge"
        case 4: return "photo"
        case 5: return "photo"
        case 6: return "sparkles"
        default: return "questionmark.square.dashed"
        }
    }
}

/// Metadata and the apply controls for the selected wallpaper.
struct DetailsPane: View {
    @EnvironmentObject var model: AppModel
    let entry: LibraryEntry

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                header
                Divider()
                applyControls

                if !model.properties.isEmpty {
                    Divider()
                    PropertiesPane(properties: model.properties)
                }
            }
            .padding(18)
        }
    }

    // MARK: Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            preview
                .frame(maxWidth: .infinity)
                .frame(height: 150)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .strokeBorder(.separator, lineWidth: 0.5)
                )

            Text(entry.title)
                .font(.title3.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 6) {
                MetadataChip(text: entry.typeLabel, systemImage: typeIcon)
                if entry.isBuiltin {
                    MetadataChip(text: "Built-in", systemImage: "shippingbox.fill")
                }
            }

            // The metadata the user asked to see on selection.
            MetadataRow(label: "Author", value: entry.author.isEmpty ? "Unknown" : entry.author)
            if !entry.description.isEmpty {
                MetadataRow(label: "Description", value: entry.description, wraps: true)
            }
            MetadataRow(label: "Format", value: typeDescription)
            if !entry.license.isEmpty {
                MetadataRow(label: "License", value: entry.license)
            }
        }
    }

    /// Shown large above the title so the selection is obvious at a glance.
    private var preview: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8)
                .fill(
                    LinearGradient(
                        colors: [Color.secondary.opacity(0.22), Color.secondary.opacity(0.07)],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

            if let path = entry.thumbnailPath, let image = NSImage(contentsOfFile: path) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: typeIcon)
                    .font(.system(size: 38))
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var typeIcon: String {
        switch entry.wallpaperType {
        case 0: return "film"
        case 1, 2: return "globe"
        case 3: return "app.badge"
        case 4, 5: return "photo"
        case 6: return "sparkles"
        default: return "questionmark.square.dashed"
        }
    }

    private var typeDescription: String {
        switch entry.wallpaperType {
        case 0: return "Video"
        case 1: return "Web page"
        case 2: return "WebGL"
        case 3: return "App"
        case 4: return "Animated GIF"
        case 5: return "Image"
        case 6: return "Metal shader"
        default: return "Unknown"
        }
    }

    // MARK: Apply

    private var applyControls: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Apply to")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            Picker("Screen", selection: $model.targetScreen) {
                ForEach(model.screens) { screen in
                    Text("Screen \(screen.index + 1) — \(screen.name)")
                        .tag(screen.index)
                }
            }
            .labelsHidden()

            Button {
                model.applySelected()
            } label: {
                Label("Apply Wallpaper", systemImage: "checkmark.circle.fill")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(model.screens.isEmpty)
            .keyboardShortcut(.defaultAction)

            if let playback = model.playback, let label = playback.label {
                // Only surfaced when there is something to say; a healthy
                // playing video needs no badge.
                Label(label, systemImage: playbackIcon(playback.state))
                    .font(.caption)
                    .foregroundStyle(playback.state == 5 ? .red : .secondary)
            }

            if let applied = appliedScreens, !applied.isEmpty {
                Label(
                    applied.count == 1
                        ? "Currently on Screen \(applied[0] + 1)"
                        : "Currently on \(applied.count) screens",
                    systemImage: "checkmark.circle.fill"
                )
                .font(.caption)
                .foregroundStyle(.tint)
            }
        }
    }

    private func playbackIcon(_ state: Int) -> String {
        switch state {
        case 1: return "arrow.down.circle"
        case 2: return "checkmark.circle"
        case 4: return "pause.circle"
        case 5: return "exclamationmark.triangle.fill"
        case 6: return "stop.circle"
        default: return "info.circle"
        }
    }

    /// Which screens this wallpaper is currently on.
    private var appliedScreens: [UInt32]? {
        let matches = model.screens.filter { $0.currentId == entry.id }.map(\.index)
        return matches.isEmpty ? nil : matches
    }
}

/// A small labelled pill, used for the type and origin badges.
struct MetadataChip: View {
    let text: String
    let systemImage: String

    var body: some View {
        Label(text, systemImage: systemImage)
            .font(.caption2)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(Color.secondary.opacity(0.16), in: Capsule())
    }
}

/// One label/value pair in the metadata list.
struct MetadataRow: View {
    let label: String
    let value: String
    /// Description text wraps; short values stay on one line.
    var wraps = false

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
            Text(value)
                .font(.callout)
                .foregroundStyle(wraps ? .primary : .primary)
                .lineLimit(wraps ? nil : 1)
                .fixedSize(horizontal: false, vertical: wraps)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct PropertiesPane: View {
    @EnvironmentObject var model: AppModel
    let properties: [PropertyEntry]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Wallpaper Options")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            ForEach(properties) { property in
                PropertyRow(property: property)
            }
        }
    }
}

struct PropertyRow: View {
    @EnvironmentObject var model: AppModel
    let property: PropertyEntry

    var body: some View {
        switch property.propType {
        case "slider":
            sliderRow
        case "checkbox":
            checkboxRow
        case "dropdown":
            dropdownRow
        case "color":
            colorRow
        default:
            // Unknown or unsupported widget: show the value read-only rather
            // than dropping it silently.
            LabeledContent(property.label) {
                Text(property.value?.stringValue ?? "—")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var sliderRow: some View {
        let current = property.value?.doubleValue ?? 0
        let lower = property.min ?? 0
        let upper = property.max ?? 100

        return VStack(alignment: .leading, spacing: 4) {
            LabeledContent(property.label) {
                Text(format(current))
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            Slider(
                value: Binding(
                    get: { current },
                    set: { _ in }
                ),
                in: lower...max(upper, lower + 0.0001),
                step: property.step ?? 1,
                onEditingChanged: { editing in
                    if !editing { model.setProperty(property.key, current) }
                }
            )
        }
    }

    private var checkboxRow: some View {
        Toggle(
            property.label,
            isOn: Binding(
                get: { property.value?.boolValue ?? false },
                set: { model.setProperty(property.key, $0) }
            )
        )
    }

    private var dropdownRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(property.label)
                .font(.callout)
            Picker(property.label, selection: Binding(
                get: { property.value?.stringValue ?? "" },
                set: { model.setProperty(property.key, $0) }
            )) {
                ForEach(property.options, id: \.self) { option in
                    Text(option).tag(option)
                }
            }
            .labelsHidden()
        }
    }

    private var colorRow: some View {
        LabeledContent(property.label) {
            HStack(spacing: 6) {
                Circle()
                    .fill(Color(hex: property.value?.stringValue ?? "#000000"))
                    .frame(width: 14, height: 14)
                Text(property.value?.stringValue ?? "—")
                    .font(.caption)
                    .monospaced()
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func format(_ value: Double) -> String {
        value == value.rounded() ? String(Int(value)) : String(format: "%.2f", value)
    }
}

// MARK: - Empty state

struct EmptyLibraryView: View {
    let onImport: (String) -> Void

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "rectangle.stack.badge.plus")
                .font(.system(size: 42))
                .foregroundStyle(.tertiary)
            Text("No wallpapers yet")
                .font(.title3.weight(.medium))
            Text("Import a video, an HTML page, an image, or a Lively project folder.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 280)
            Button("Import Wallpaper…", action: chooseFile)
                .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = "Select a media file or a Lively project folder"
        if panel.runModal() == .OK, let url = panel.url {
            onImport(url.path)
        }
    }

}

// MARK: - Helpers

extension Color {
    /// `#rrggbb` / `#rgb` from a wallpaper's colour property.
    init(hex: String) {
        var value = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.hasPrefix("#") { value.removeFirst() }

        if value.count == 3 {
            value = value.map { "\($0)\($0)" }.joined()
        }

        guard value.count == 6, let rgb = UInt32(value, radix: 16) else {
            self = .gray
            return
        }

        self.init(
            .sRGB,
            red: Double((rgb >> 16) & 0xff) / 255,
            green: Double((rgb >> 8) & 0xff) / 255,
            blue: Double(rgb & 0xff) / 255,
            opacity: 1
        )
    }
}

// MARK: - Import

/// The open panel is created here rather than in the model, so the view layer
/// owns the one piece that is genuinely a UI concern.
@MainActor
func presentImportPanel() {
    let panel = NSOpenPanel()
    panel.canChooseFiles = true
    panel.canChooseDirectories = true
    panel.allowsMultipleSelection = false
    panel.message = "Select a media file or a Lively project folder"

    guard panel.runModal() == .OK, let url = panel.url else { return }
    AppModel.shared.importWallpaper(at: url.path)
}
