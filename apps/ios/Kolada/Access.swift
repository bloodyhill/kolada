// VoiceOver for the note: it reads the note line by line and page by page
// (UIAccessibilityReadingContent), knows it is text being edited, and hears
// of each change. The answers come from the model (crates/kolada-ios, the
// model's access calls, as the Mac's VoiceOver gets them).

import UIKit

extension NoteView: UIAccessibilityReadingContent {
    override var isAccessibilityElement: Bool { get { true } set {} }

    override var accessibilityLabel: String? {
        get {
            guard let s = kolada_label(app) else { return nil }
            defer { kolada_free_string(s) }
            return String(cString: s)
        }
        set {}
    }

    override var accessibilityValue: String? {
        get { text(in: Range(0, kolada_length(app))) }
        set {}
    }

    override var accessibilityTraits: UIAccessibilityTraits {
        get { [.allowsDirectInteraction, .updatesFrequently] }
        set {}
    }

    func accessibilityLineNumber(for point: CGPoint) -> Int {
        kolada_line_of(app, kolada_index_at(app, model(point).x, model(point).y))
    }

    func accessibilityContent(forLineNumber lineNumber: Int) -> String? {
        var location = 0, length = 0
        guard lineNumber >= 0, kolada_line_range(app, lineNumber, &location, &length) else { return nil }
        return text(in: Range(location, location + length))
    }

    func accessibilityFrame(forLineNumber lineNumber: Int) -> CGRect {
        var location = 0, length = 0
        guard lineNumber >= 0, kolada_line_range(app, lineNumber, &location, &length) else { return .zero }
        let r = firstRect(for: Range(location, location + length))
        return r.isNull ? .zero : UIAccessibility.convertToScreenCoordinates(r, in: self)
    }

    func accessibilityPageContent() -> String? {
        var location = 0, length = 0
        kolada_visible(app, &location, &length)
        return text(in: Range(location, location + length))
    }

    /// VoiceOver hears the note changed (typing, a paste, undo).
    func announceChange() {
        guard UIAccessibility.isVoiceOverRunning else { return }
        UIAccessibility.post(notification: .layoutChanged, argument: nil)
    }

    /// VoiceOver's two-finger scrub: what Escape does (a menu, a card, search closes).
    override func accessibilityPerformEscape() -> Bool {
        var closed = false
        outside {
            closed = kolada_do(app, "escape")
            return closed
        }
        return closed
    }
}

/// The chrome's controls for VoiceOver (the side menu's rows and buttons,
/// the toolbar, settings, menus and cards), named by the model from what
/// each frame drew (`kolada_controls`). Lies over the note view, takes no
/// touches; a control activated is tapped in the note view, as a finger would.
final class ControlsView: UIView {
    weak var note: NoteView?
    private var elements: [Control]?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        isOpaque = false
        backgroundColor = .clear
    }

    required init?(coder: NSCoder) { fatalError("made in code") }

    override var isAccessibilityElement: Bool { get { false } set {} }

    /// A frame was drawn: the names are read again when VoiceOver next asks.
    func drawn() {
        elements = nil
    }

    override var accessibilityElements: [Any]? {
        get {
            if let elements { return elements }
            guard let note, let lines = kolada_controls(note.app) else { return [] }
            defer { kolada_free_string(lines) }
            let made = String(cString: lines).split(separator: "\n").compactMap { line -> Control? in
                let f = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
                guard f.count == 6, let l = Float(f[0]), let t = Float(f[1]), let r = Float(f[2]), let b = Float(f[3]) else { return nil }
                let c = Control(accessibilityContainer: self)
                c.note = note
                c.accessibilityFrameInContainerSpace = note.view([l, t, r, b])
                c.accessibilityLabel = f[5]
                switch f[4] {
                case "selected": c.accessibilityTraits = [.button, .selected]
                case "heading": c.accessibilityTraits = [.button, .header]
                case "field": c.accessibilityTraits = [.searchField]
                case "task-done":
                    c.accessibilityTraits = [.button, .selected]
                    c.accessibilityValue = "Done"
                case "task":
                    c.accessibilityTraits = [.button]
                    c.accessibilityValue = "Not done"
                default: c.accessibilityTraits = [.button]
                }
                return c
            }
            elements = made
            return made
        }
        set {}
    }
}

/// One of the chrome's controls: activated, the note view is tapped at its middle.
final class Control: UIAccessibilityElement {
    weak var note: NoteView?

    override func accessibilityActivate() -> Bool {
        guard let note else { return false }
        let r = accessibilityFrameInContainerSpace
        note.tap(at: CGPoint(x: r.midX, y: r.midY), flags: [])
        UIAccessibility.post(notification: .layoutChanged, argument: nil)
        return true
    }
}
