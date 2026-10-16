import SwiftUI
import AppKit

// MARK: - Golden Gate

/// The "Golden Gate" tint from Apple's Liquid Glass palette.
///
/// Warm amber over a neutral dark base. The hue is what Apple's system picks
/// for that name, nudged slightly warmer and less saturated so it reads well
/// against translucent panels rather than fighting them.
enum GoldenGate {
    /// The accent used for tint, prominent buttons and the active selection.
    static let tint = Color(
        red: 0.98,
        green: 0.64,
        blue: 0.24
    )

    /// Identity for glass merging.
    var id: String { "GoldenGate" }

    /// Deeper amber for text and icons that need to hold up on light glass.
    static let deep = Color(
        red: 0.76,
        green: 0.42,
        blue: 0.10
    )

    /// A wide, soft gradient for large surfaces. Liquid Glass works because the
    /// material picks up colour from behind it, so the backdrop needs something
    /// to pick up.
    static var backdrop: LinearGradient {
        LinearGradient(
            colors: [
                Color(red: 0.16, green: 0.13, blue: 0.11),
                Color(red: 0.09, green: 0.08, blue: 0.10),
                Color(red: 0.13, green: 0.10, blue: 0.08)
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    /// The highlight that gives glass its lit-from-above look.
    static var sheen: LinearGradient {
        LinearGradient(
            colors: [.white.opacity(0.16), .white.opacity(0.02), .clear],
            startPoint: .top,
            endPoint: .bottom
        )
    }
}

/// The backdrop the glass sits against.
///
/// Liquid Glass is not a colour, it is a refraction: it samples whatever is
/// behind it and bends it. Against a flat fill it collapses into an ordinary
/// dark rectangle no matter how the material is configured -- which is exactly
/// what happened here at first.
///
/// So the background carries actual light sources. Warm Golden Gate bloom on
/// the left where the sidebar is, a cooler bloom on the right, and a faint one
/// low in the middle, so every panel has something worth refracting.
struct AuroraBackdrop: View {
    var body: some View {
        ZStack {
            Color.black

            RadialGradient(
                colors: [GoldenGate.tint.opacity(0.20), .clear],
                center: UnitPoint(x: 0.08, y: 0.04),
                startRadius: 20,
                endRadius: 520
            )

            RadialGradient(
                colors: [
                    Color(red: 0.34, green: 0.58, blue: 1.0).opacity(0.26),
                    .clear
                ],
                center: UnitPoint(x: 0.92, y: 0.12),
                startRadius: 20,
                endRadius: 640
            )

            RadialGradient(
                colors: [GoldenGate.deep.opacity(0.14), .clear],
                center: UnitPoint(x: 0.58, y: 1.04),
                startRadius: 20,
                endRadius: 700
            )

            RadialGradient(
                colors: [Color.white.opacity(0.07), .clear],
                center: UnitPoint(x: 0.5, y: -0.05),
                startRadius: 10,
                endRadius: 420
            )
        }
        .ignoresSafeArea()
    }
}

// MARK: - Glass

/// The Liquid Glass material, or the nearest thing to it on older systems.
///
/// The real APIs exist from macOS 26. Rather than scatter `if #available`
/// through every view, they are concentrated here: on macOS 26 and newer this
/// is genuine Liquid Glass; below that it degrades to a material that reads
/// similarly enough to still look intentional.
///
/// This returns *only the background shape*. It must be used through
/// `.background { }`, never applied to the content directly -- returning it
/// as a plain view silently discards whatever it was chained onto.
@ViewBuilder
func glassBackdrop(_ shape: some Shape, tint: Color?, interactive: Bool) -> some View {
    if #available(macOS 26.0, *) {
        // `.interactive()` makes the glass react to pointer and keyboard focus.
        let effect = interactive ? Glass.regular.interactive() : Glass.regular
        let glass = shape.fill(.clear).glassEffect(effect, in: shape)
        if let tint {
            glass.overlay(shape.fill(tint.opacity(0.18)))
        } else {
            glass
        }
    } else {
        let base = shape.fill(.ultraThinMaterial)
        if let tint {
            base.overlay(shape.fill(tint.opacity(0.12)))
        } else {
            base
        }
    }
}

extension View {
    /// Liquid Glass behind this view, on a rounded rectangle.
    func glassCard(
        cornerRadius: CGFloat = 14,
        tint: Color? = nil,
        interactive: Bool = false
    ) -> some View {
        background {
            glassBackdrop(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous),
                tint: tint,
                interactive: interactive
            )
        }
    }

    /// A hairline edge, brightest along the top. This is what separates glass
    /// from a flat grey box when there is little behind it to refract.
    func glassEdge(cornerRadius: CGFloat, highlighted: Bool) -> some View {
        overlay(
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .strokeBorder(
                    highlighted ? GoldenGate.tint : .white.opacity(0.10),
                    lineWidth: highlighted ? 2 : 1
                )
        )
    }
}

// MARK: - Selection

/// The background for the active sidebar row.
///
/// Kept here so the availability check lives in one place; a `#available` in
/// the middle of the sidebar layout would be noise in a view that is otherwise
/// just arrangement.
struct ActiveRowBackground: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 11, style: .continuous)
            .fill(.clear)
            .modifier(LiquidGlass())
    }
}

private struct LiquidGlass: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 26.0, *) {
            content.glassEffect(
                .regular.tint(GoldenGate.tint.opacity(0.3)),
                in: .rect(cornerRadius: 11)
            )
        } else {
            content.background(
                RoundedRectangle(cornerRadius: 11, style: .continuous)
                    .fill(GoldenGate.tint.opacity(0.22))
            )
        }
    }
}

// MARK: - Panels

/// A standard content panel: glass, a soft top highlight, and a hairline edge.
struct GlassPanel<Content: View>: View {
    var cornerRadius: CGFloat = 16
    var tint: Color?
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassCard(cornerRadius: cornerRadius, tint: tint)
            // The lit top edge is what separates glass from a flat grey box.
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(
                        LinearGradient(
                            colors: [.white.opacity(0.22), .white.opacity(0.05), .clear],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        lineWidth: 1
                    )
            )
            .shadow(color: .black.opacity(0.35), radius: 14, y: 6)
    }
}

/// A small caption that sits above a panel or a group of controls.
struct Eyebrow: View {
    let title: String
    var systemImage: String?
    var tint: Color = GoldenGate.deep

    var body: some View {
        HStack(spacing: 5) {
            if let systemImage {
                Image(systemName: systemImage)
            }
            Text(title.uppercased())
                .tracking(0.6)
        }
        .font(.system(size: 10, weight: .semibold))
        .foregroundStyle(tint)
    }
}

// MARK: - Motion

extension AnyView {
    /// The app's standard response to a value changing.
    func livelyChange() -> AnyView {
        AnyView(
            self.animation(
                .spring(response: 0.34, dampingFraction: 0.82),
                value: UUID()
            )
        )
    }
}

enum LivelyMotion {
    /// Panels settling in.
    static let panel = Animation.spring(response: 0.38, dampingFraction: 0.86)
    /// Selection changes.
    static let select = Animation.spring(response: 0.28, dampingFraction: 0.78)
    /// Hover and press feedback.
    static let nudge = Animation.spring(response: 0.22, dampingFraction: 0.7)
}

// MARK: - Toolbar

extension View {
    /// Applies the Golden Gate tint to every control in a subtree.
    func goldenGateTint() -> some View {
        tint(GoldenGate.tint)
    }

    /// The app's standard push button.
    ///
    /// `@ViewBuilder` because the two branches have different opaque types and
    /// the compiler cannot reconcile them otherwise.
    @ViewBuilder
    func livelyPrimaryButton() -> some View {
        if #available(macOS 26.0, *) {
            self.buttonStyle(.glassProminent)
        } else {
            self.buttonStyle(.borderedProminent)
        }
    }

    /// The app's standard secondary button.
    @ViewBuilder
    func livelySecondaryButton() -> some View {
        if #available(macOS 26.0, *) {
            self.buttonStyle(.glass)
        } else {
            self.buttonStyle(.bordered)
        }
    }
}