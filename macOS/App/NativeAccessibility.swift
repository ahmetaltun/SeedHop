import SwiftUI
import AppKit

/// Suppress identical updates without suppressing a later recurrence after clearing.
struct AccessibilityAnnouncementGate {
    private var previous: String?
    mutating func next(_ message: String?) -> String? {
        let value = message?.trimmingCharacters(in:.whitespacesAndNewlines)
        let normalized = value?.isEmpty == false ? value : nil
        defer { previous = normalized }
        return normalized == previous ? nil : normalized
    }
}

private struct AccessibilityAnnouncementsKey: EnvironmentKey {
    static let defaultValue = true
}
extension EnvironmentValues {
    var accessibilityAnnouncementsEnabled: Bool {
        get { self[AccessibilityAnnouncementsKey.self] }
        set { self[AccessibilityAnnouncementsKey.self] = newValue }
    }
}

@MainActor enum NativeAccessibility {
    #if SEEDHOP_DEVELOPMENT
    /// The native fixture observes requests instead of interrupting assistive speech.
    static var announcementObserver: ((String, NSAccessibilityPriorityLevel) -> Void)?
    #endif
    static func announce(_ message: String, priority: NSAccessibilityPriorityLevel = .high) {
        guard NSApp.isActive, let window = NSApp.keyWindow, window.isVisible else { return }
        #if SEEDHOP_DEVELOPMENT
        if let announcementObserver { announcementObserver(message,priority); return }
        #endif
        NSAccessibility.post(element:window,notification:.announcementRequested,userInfo:[
            .announcement:message,
            .priority:priority.rawValue
        ])
    }
    static func restore(_ responder: NSResponder?, in window: NSWindow) {
        guard let view = responder as? NSView, view.window === window else { return }
        window.makeFirstResponder(view)
    }
}

private struct AccessibilityAnnouncement: ViewModifier {
    let message: String?
    var priority: NSAccessibilityPriorityLevel
    @Environment(\.accessibilityAnnouncementsEnabled) private var enabled
    @State private var gate = AccessibilityAnnouncementGate()
    func body(content: Content) -> some View {
        content.onChange(of:message,initial:true) { _, message in
            guard enabled, let announcement = gate.next(message) else { return }
            NativeAccessibility.announce(announcement,priority:priority)
        }
    }
}
extension View {
    func announceAccessibility(_ message: String?, priority: NSAccessibilityPriorityLevel = .high) -> some View {
        modifier(AccessibilityAnnouncement(message:message,priority:priority))
    }
}

