import SwiftUI

// MARK: - Root

struct ContentView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        NavigationSplitView {
            sidebar
                .navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 260)
        } detail: {
            // Previously this always rendered the library, so even a working
            // sidebar selection would not have changed the page.
            switch model.section {
            case .library: libraryView
            case .screens: ScreensView()
            case .settings: SettingsView()
            }
        }
        .background { AuroraBackdrop() }
        .toolbar { toolbarContent }
        .goldenGateTint()
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
        List {
            Section("Library") {
                sidebarButton("Library", "square.grid.2x2", .library, count: model.library.count)
                sidebarButton("Screens", "display", .screens)
                sidebarButton("Settings", "gearshape", .settings)
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

    /// One sidebar row.
    ///
    /// A plain Button rather than `NavigationLink(value:)`: there is no
    /// `NavigationStack` in this window, so a value-based link had nowhere to
    /// push and silently did nothing.
    private func sidebarButton(
        _ title: String,
        _ icon: String,
        _ target: SidebarPage,
        count: Int? = nil
    ) -> some View {
        let active = model.section == target
        return Button {
            model.go(target)
        } label: {
            HStack(spacing: 9) {
                Image(systemName: icon).frame(width: 18)
                Text(title).font(.callout.weight(active ? .semibold : .regular))
                Spacer(minLength: 6)
                if let count {
                    Text("\(count)")
                        .font(.caption2.monospacedDigit())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(.white.opacity(0.10)))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .foregroundStyle(active ? GoldenGate.tint : .primary)
        }
        .buttonStyle(.plain)
        .listRowBackground(
            Group {
                if active {
                    // The active page is a glass capsule, not a flat bar. This
                    // is the single strongest cue about where you are.
                    ActiveRowBackground()
                }
            }
        )
        .overlay(alignment: .leading) {
            // Outside the row's content, not on top of the icon. The row
            // reserves the inset itself so the glyphs stay aligned.
            if active {
                Capsule()
                    .fill(GoldenGate.tint)
                    .frame(width: 3, height: 16)
                    .padding(.leading, 3)
            }
        }
        .foregroundStyle(active ? GoldenGate.tint : .primary)
        .accessibilityAddTraits(active ? [.isSelected] : [])
    }

    private var sidebarSelection: Binding<Selection?> {
        Binding(
            get: {
                switch model.section {
                case .library: return model.selectedId.map { Selection.item($0) } ?? .library
                case .screens: return .screens
                case .settings: return .settings
                }
            },
            set: { newValue in
                switch newValue {
                case .library: model.go(.library)
                case .screens: model.go(.screens)
                case .settings: model.go(.settings)
                case .item(let id):
                    // Selecting a wallpaper means "show the library, with this
                    // one selected" -- otherwise clicking a card would leave the
                    // user stranded on the Screens page.
                    model.go(.library)
                    guard let entry = model.library.first(where: { $0.id == id }) else { return }
                    model.select(entry)
                case .none: break
                }
            }
        )
    }

    private var libraryView: some View {
        HSplitView {
            libraryColumn
                .frame(minWidth: 300, idealWidth: 380)

            rightColumn
                .frame(minWidth: 280, idealWidth: 320)
        }
        .frame(minWidth: 640, idealWidth: 700)
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
        detailBody
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassCard(cornerRadius: 18)
            .glassEdge(cornerRadius: 18, highlighted: false)
            .shadow(color: .black.opacity(0.4), radius: 26, y: 10)
    }

    private var detailBody: some View {
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
                CommitSlider(
                    key: "__volume",
                    value: Double(config.volumePercent),
                    range: 0...100,
                    step: 1,
                    width: 110,
                    onCommit: { model.setVolume($0) }
                )
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
        VStack(alignment: .leading, spacing: 8) {
            thumbnail

            Text(entry.title)
                .font(.callout.weight(.semibold))
                .lineLimit(1)

            Text(entry.description)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)

            Spacer(minLength: 0)

            HStack(spacing: 6) {
                Label(entry.traitBadge, systemImage: entry.symbol)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                Spacer(minLength: 6)
                Button("Set Wallpaper") { onTap() }
                    .font(.caption2.weight(.medium))
                    .controlSize(.small)
                    .disabled(isApplied)
            }
        }
        .padding(10)
        .frame(height: 224, alignment: .top)
        .glassCard(cornerRadius: 13, tint: isSelected ? GoldenGate.tint : nil)
        .glassEdge(cornerRadius: 13, highlighted: isSelected)
        .shadow(color: isSelected ? GoldenGate.tint.opacity(0.30) : .clear, radius: 14, y: 4)
        .contentShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
        .animation(LivelyMotion.select, value: isSelected)
        .onTapGesture(perform: onTap)
    }

    private var thumbnail: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 8)
                .fill(
                    LinearGradient(
                        colors: [Color.secondary.opacity(0.20), Color.secondary.opacity(0.06)],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

            if let path = entry.thumbnailPath,
               let image = NSImage(contentsOfFile: path) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .clipped()
            } else {
                VStack(spacing: 5) {
                    Image(systemName: entry.symbol).font(.system(size: 22, weight: .light))
                    Text(entry.typeLabel).font(.caption2.weight(.medium))
                }
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }

            HStack {
                // The wallpaper's own declared renderer, never a guess.
                Text(entry.engineBadge)
                    .font(.system(size: 9, weight: .semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(.black.opacity(0.55)))
                    .foregroundStyle(.white.opacity(0.9))
                Spacer()
                if isApplied {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(GoldenGate.tint)
                        .help("Currently applied")
                }
            }
            .padding(7)
        }
        .frame(height: 100)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.white.opacity(0.10), lineWidth: 1))
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
                    PropertiesPane(properties: model.properties, propertiesOnlyWeb: true)
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
    var propertiesOnlyWeb: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Wallpaper Options")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            ForEach(properties) { property in
                PropertyRow(property: property)
            }

            // Properties are only implemented for web wallpapers. A slider
            // the engine will reject is worse than no slider, so the panel
            // says so rather than offering a control that cannot work.
            if propertiesOnlyWeb, !model.isWebWallpaper {
                Text("Applies to web wallpapers. This one is a \(model.selectedTypeName) wallpaper.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
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
            CommitSlider(
                key: property.key,
                value: current,
                range: lower...max(upper, lower + 0.0001),
                step: property.step ?? 1,
                onCommit: { model.setProperty(property.key, $0) }
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

    /// A colour needs an editor, not a swatch. This row displayed the value
    /// with no way to change it, so any package shipping a `color` property
    /// got a control the user could not touch.
    private var colorRow: some View {
        LabeledContent(property.label) {
            HStack(spacing: 6) {
                ColorPicker(
                    "",
                    selection: Binding(
                        get: { Color(hex: property.value?.stringValue ?? "#000000") },
                        set: { model.setProperty(property.key, $0.hexString ?? "#000000") }
                    ),
                    supportsOpacity: false
                )
                .labelsHidden()
                .frame(width: 20)

                Text(property.value?.stringValue ?? "—")
                    .font(.caption)
                    .monospaced()
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
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
                .livelyPrimaryButton()
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

    /// `#rrggbb`, for handing an edited colour back to the daemon.
    ///
    /// Routed through NSColor so the conversion uses a defined colour space
    /// rather than assuming which components SwiftUI happens to be holding.
    var hexString: String? {
        guard let c = NSColor(self).usingColorSpace(.sRGB) else { return nil }
        return String(format: "#%02x%02x%02x",
                      Int((c.redComponent * 255).rounded()),
                      Int((c.greenComponent * 255).rounded()),
                      Int((c.blueComponent * 255).rounded()))
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

/// A slider that moves while you drag it and commits once on release.
///
/// Binding straight to the daemon's value cannot work. That value only changes
/// when a fresh snapshot arrives from the round trip, so during a drag the
/// getter keeps returning the old number and the thumb springs back to where it
/// started -- the control appears completely dead. Both the master volume
/// slider and every wallpaper property slider were written this way.
///
/// The draft holds the in-flight value; committing clears it so the control
/// goes back to following the daemon, including any correction it makes.
struct CommitSlider: View {
    let key: String
    let value: Double
    let range: ClosedRange<Double>
    let step: Double
    var width: CGFloat?
    let onCommit: (Double) -> Void

    @EnvironmentObject var model: AppModel

    var body: some View {
        Slider(
            value: Binding(
                get: { model.draft(key, fallback: value) },
                set: { model.setDraft(key, $0) }
            ),
            in: range,
            step: step,
            onEditingChanged: { editing in
                guard !editing else { return }
                let pending = model.draft(key, fallback: value)
                model.clearDraft(key)
                onCommit(pending)
            }
        )
        .frame(width: width)
        // The daemon has the final word: if it corrects or clamps the value,
        // follow it once the drag is over.
        .onChange(of: value) { _, _ in model.clearDraft(key) }
    }
}

// MARK: - Screens

/// Per-monitor overview.
///
/// Shows what is actually on each display right now, with the actions that
/// only make sense per display: change it, clear it, or push one wallpaper to
/// every screen at once.
struct ScreensView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                header

                if model.screens.isEmpty {
                    empty
                } else {
                    ForEach(model.screens) { screen in
                        ScreenCard(screen: screen)
                    }

                    // Always present, even with one display. Hiding it left a
                    // single-screen page looking broken and unfinished.
                    applyToAll
                }

                diagnosticsHint
            }
            // Without a width cap the controls stretch the full width of a
            // maximised window and read as an unfinished form.
            .frame(maxWidth: 560, alignment: .leading)
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .frame(maxWidth: .infinity, alignment: .top)
        }
        .frame(minWidth: 560, idealWidth: 640)
        .onAppear { model.reload() }
    }

    private var header: some View {
        SectionHeader(
            title: "Screens",
            subtitle: model.screens.isEmpty
                ? "No displays reported"
                : "\(model.screens.count) display\(model.screens.count == 1 ? "" : "s") attached",
            systemImage: "display"
        )
    }

    private var empty: some View {
        VStack(spacing: 10) {
            Image(systemName: "display.trianglebadge.exclamationmark")
                .font(.system(size: 30, weight: .light))
                .foregroundStyle(.tertiary)
            Text("No screens detected")
                .font(.callout.weight(.medium))
            Text("The daemon has not reported any displays. Try `lively diagnostics`.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 44)
    }

    private var applyToAll: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(
                title: "All screens",
                subtitle: "One wallpaper everywhere",
                systemImage: "rectangle.on.rectangle"
            )
            Button {
                model.applySelectedToAll()
            } label: {
                Label(
                    model.selectedId == nil
                        ? "Select a wallpaper first"
                        : "Apply “\(model.selectedTitle)” to every screen",
                    systemImage: "arrow.triangle.2.circlepath"
                )
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .controlSize(.large)
            .disabled(model.selectedId == nil)
            Text("Applies the wallpaper currently selected in the library.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(16)
        .glassCard(cornerRadius: 16)
    }

    private var diagnosticsHint: some View {
        HStack(spacing: 6) {
            Image(systemName: "info.circle")
            Text("Run `lively diagnostics` to see what the backend supports on this machine.")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

/// One display, as a card.
private struct ScreenCard: View {
    @EnvironmentObject var model: AppModel
    let screen: ScreenEntry

    static let background = RoundedRectangle(cornerRadius: 12)
        .fill(Color(nsColor: .controlBackgroundColor))

    private var ratio: CGFloat {
        CGFloat(screen.width) / CGFloat(max(screen.height, 1))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 16) {
                thumbnail
                details
                Spacer(minLength: 0)
            }
            .padding(16)

            Divider().opacity(0.4)

            actions
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
        }
        .glassCard(cornerRadius: 16)
    }

    /// A rectangle at the display's real aspect ratio, filled with the current
    /// wallpaper's thumbnail when there is one.
    private var thumbnail: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 6)
                .fill(.quaternary)
                .aspectRatio(ratio, contentMode: .fit)
                .frame(width: 116)
                .overlay {
                    Image(systemName: screen.currentId == nil ? "rectangle.slash" : "photo")
                        .font(.system(size: 20, weight: .light))
                        .foregroundStyle(.tertiary)
                }
        }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("Screen \(screen.index + 1)")
                .font(.callout.weight(.semibold))
            Text(screen.name)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text("\(screen.width) × \(screen.height)")
                .font(.caption.monospacedDigit())
                .foregroundStyle(.tertiary)

            HStack(spacing: 5) {
                Circle()
                    .fill(screen.currentId == nil ? Color.secondary.opacity(0.5) : Color.green)
                    .frame(width: 6, height: 6)
                Text(screen.currentTitle ?? "No wallpaper set")
                    .font(.callout)
                    .foregroundStyle(screen.currentId == nil ? .secondary : .primary)
                    .lineLimit(1)
            }
            .padding(.top, 3)
        }
    }

    private var actions: some View {
        HStack(spacing: 8) {
            Button("Use selected") { model.applySelected(to: screen.index) }
                .disabled(model.selectedId == nil)
            Button("Clear") { model.clearScreen(screen.index) }
                .disabled(screen.currentId == nil)
            Spacer()
        }
        .controlSize(.small)
        .buttonStyle(.borderless)
    }
}

// MARK: - Settings

/// Preferences that apply to the whole engine rather than one wallpaper.
struct SettingsView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                SectionHeader(
                    title: "Settings",
                    subtitle: "Applies to every wallpaper",
                    systemImage: "gearshape"
                )

                if let config = model.config {
                    playback(config)
                    audio(config)
                    display(config)
                    about
                } else {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Loading settings…").foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 30)
                }
            }
            .frame(maxWidth: 560, alignment: .leading)
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .frame(maxWidth: .infinity, alignment: .top)
        }
        .frame(minWidth: 560, idealWidth: 640)
        .onAppear { model.reload() }
    }

    private func playback(_ config: ConfigView) -> some View {
        group("Playback", systemImage: "play.circle") {
            toggle("Pause during a fullscreen app",
                   value: config.pauseOnFullscreen,
                   key: "pauseOnFullscreen")
            toggle("Pause on battery power",
                   value: config.pauseOnBattery,
                   key: "pauseOnBattery")
            toggle("Paused", value: config.isPaused, key: "isPaused")
        }
    }

    private func audio(_ config: ConfigView) -> some View {
        group("Audio", systemImage: "speaker.wave.2") {
            toggle("Muted", value: config.isMuted, key: "isMuted")
            toggle("Audio visualiser",
                   value: config.audioVisualizerEnabled,
                   key: "audioVisualizerEnabled")
            HStack {
                Text("Volume")
                Spacer()
                Text("\(config.volumePercent)%")
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            CommitSlider(
                key: "__volume",
                value: Double(config.volumePercent),
                range: 0...100,
                step: 1,
                onCommit: { model.setVolume($0) }
            )
            .controlSize(.small)
        }
    }

    private func display(_ config: ConfigView) -> some View {
        group("Displays", systemImage: "rectangle.3.group") {
            Picker("Layout", selection: Binding(
                get: { config.layoutMode },
                set: { model.setConfig("layoutMode", $0) }
            )) {
                Text("Per screen").tag(0)
                Text("Span all screens").tag(1)
                Text("Duplicate").tag(2)
            }
            .pickerStyle(.radioGroup)

            Text("Only takes effect once more than one display is attached.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var configPath: String { model.config?.path ?? "" }

    private var about: some View {
        group("About", systemImage: "info.circle") {
            row("Version", Bundle.main.appVersion ?? "dev")
            row("Renderers", "Image · Video · Web · URL · Shader")
            row("Settings file", configPath)
        }
    }

    // MARK: Building blocks

    private func toggle(_ title: String, value: Bool, key: String) -> some View {
        Toggle(title, isOn: Binding(
            get: { value },
            set: { model.setConfig(key, $0) }
        ))
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label).foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(value)
                .font(.callout)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }

    private func group<Content: View>(
        _ title: String,
        systemImage: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: systemImage)
                .font(.callout.weight(.semibold))
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 10) {
                content()
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassCard(cornerRadius: 16)
        }
    }
}

private extension Bundle {
    var appVersion: String? {
        object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    }
}
