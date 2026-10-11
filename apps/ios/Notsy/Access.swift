// VoiceOver for the note: it reads the note line by line and page by page
// (UIAccessibilityReadingContent), knows it is text being edited, and hears
// of each change. The answers come from the model (crates/notsy-ios, the
// model's access calls, as the Mac's VoiceOver gets them).

import UIKit

extension NoteView: UIAccessibilityReadingContent {
    override var isAccessibilityElement: Bool { get { true } set {} }

    override var accessibilityLabel: String? {
        get {
            guard let s = notsy_label(app) else { return nil }
            defer { notsy_free_string(s) }
            return String(cString: s)
        }
        set {}
    }

    override var accessibilityValue: String? {
        get { text(in: Range(0, notsy_length(app))) }
        set {}
    }

    override var accessibilityTraits: UIAccessibilityTraits {
        get { [.allowsDirectInteraction, .updatesFrequently] }
        set {}
    }

    func accessibilityLineNumber(for point: CGPoint) -> Int {
        notsy_line_of(app, notsy_index_at(app, Float(point.x), Float(point.y)))
    }

    func accessibilityContent(forLineNumber lineNumber: Int) -> String? {
        var location = 0, length = 0
        guard lineNumber >= 0, notsy_line_range(app, lineNumber, &location, &length) else { return nil }
        return text(in: Range(location, location + length))
    }

    func accessibilityFrame(forLineNumber lineNumber: Int) -> CGRect {
        var location = 0, length = 0
        guard lineNumber >= 0, notsy_line_range(app, lineNumber, &location, &length) else { return .zero }
        let r = firstRect(for: Range(location, location + length))
        return r.isNull ? .zero : UIAccessibility.convertToScreenCoordinates(r, in: self)
    }

    func accessibilityPageContent() -> String? {
        var location = 0, length = 0
        notsy_visible(app, &location, &length)
        return text(in: Range(location, location + length))
    }

    /// VoiceOver hears the note changed (typing, a paste, undo).
    func announceChange() {
        guard UIAccessibility.isVoiceOverRunning else { return }
        UIAccessibility.post(notification: .layoutChanged, argument: nil)
    }
}
