// UITextInput for the note: what input methods (Japanese, Chinese, Korean),
// dictation and the keyboard's own features ask of a text view. Positions
// are UTF-16 offsets, as the model counts them (crates/kolada-ios); every
// answer comes from the model.

import UIKit

final class Position: UITextPosition {
    let offset: Int
    init(_ offset: Int) { self.offset = max(0, offset) }
}

final class Range: UITextRange {
    let from: Int, to: Int
    init(_ a: Int, _ b: Int) { (from, to) = (min(a, b), max(a, b)) }
    override var start: UITextPosition { Position(from) }
    override var end: UITextPosition { Position(to) }
    override var isEmpty: Bool { from == to }
}

extension NoteView {
    private var length: Int { kolada_length(app) }

    private func offset(_ p: UITextPosition) -> Int { min((p as? Position)?.offset ?? 0, length) }

    private func cgRect(_ r: [Float]) -> CGRect {
        CGRect(x: CGFloat(r[0]), y: CGFloat(r[1]), width: CGFloat(max(r[2] - r[0], 1)), height: CGFloat(r[3] - r[1]))
    }

    // MARK: Text

    func text(in range: UITextRange) -> String? {
        guard let r = range as? Range else { return nil }
        guard let s = kolada_substring(app, r.from, r.to - r.from) else { return nil }
        defer { kolada_free_string(s) }
        return String(cString: s)
    }

    func replace(_ range: UITextRange, withText text: String) {
        guard let r = range as? Range else { return }
        kolada_select(app, r.from, r.to - r.from)
        _ = kolada_insert(app, text)
        redraw()
        edited()
    }

    var selectedTextRange: UITextRange? {
        get {
            var location = 0, length = 0
            kolada_selection(app, &location, &length)
            return Range(location, location + length)
        }
        set {
            guard let r = newValue as? Range else { return }
            kolada_select(app, r.from, r.to - r.from)
            redraw()
        }
    }

    var markedTextRange: UITextRange? {
        var location = 0, length = 0
        return kolada_marked(app, &location, &length) ? Range(location, location + length) : nil
    }

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        // The selection inside the marked text: where the caret is, and the
        // clause being converted when it has a length.
        let cursor = selectedRange.location == NSNotFound ? (markedText ?? "").utf16.count : selectedRange.location
        if kolada_set_marked(app, markedText ?? "", cursor, selectedRange.location == NSNotFound ? 0 : selectedRange.length) {
            redraw()
            edited()
        }
    }

    func unmarkText() {
        if kolada_unmark(app) { redraw() }
    }

    // MARK: Positions

    var beginningOfDocument: UITextPosition { Position(0) }
    var endOfDocument: UITextPosition { Position(length) }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        Range(offset(fromPosition), offset(toPosition))
    }

    func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        let to = self.offset(position) + offset
        return (0...length).contains(to) ? Position(to) : nil
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        switch direction {
        case .left, .up: return self.position(from: position, offset: -offset)
        default: return self.position(from: position, offset: offset)
        }
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        let (a, b) = (offset(position), offset(other))
        return a < b ? .orderedAscending : a > b ? .orderedDescending : .orderedSame
    }

    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int { offset(toPosition) - offset(from) }

    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        guard let r = range as? Range else { return nil }
        switch direction {
        case .left, .up: return Position(r.from)
        default: return Position(r.to)
        }
    }

    func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
        let p = offset(position)
        switch direction {
        case .left, .up: return Range(max(p - 1, 0), p)
        default: return Range(p, min(p + 1, length))
        }
    }

    func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection { .natural }
    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}

    // MARK: Geometry

    func firstRect(for range: UITextRange) -> CGRect {
        guard let r = range as? Range else { return .null }
        var out = [Float](repeating: 0, count: 4)
        return kolada_rect(app, r.from, r.to - r.from, &out) ? cgRect(out) : .null
    }

    func caretRect(for position: UITextPosition) -> CGRect {
        var out = [Float](repeating: 0, count: 4)
        let p = offset(position)
        if kolada_rect(app, p, 0, &out) {
            // A paragraph's box for a position not at the caret: its start.
            var r = cgRect(out)
            r.size.width = 2
            return r
        }
        return .zero
    }

    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        let rect = firstRect(for: range)
        return rect.isNull ? [] : [Selection(rect)]
    }

    func closestPosition(to point: CGPoint) -> UITextPosition? {
        Position(kolada_index_at(app, Float(point.x), Float(point.y)))
    }

    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? {
        guard let r = range as? Range else { return nil }
        return Position(min(max(offset(closestPosition(to: point) ?? Position(0)), r.from), r.to))
    }

    func characterRange(at point: CGPoint) -> UITextRange? {
        let p = offset(closestPosition(to: point) ?? Position(0))
        return Range(p, min(p + 1, length))
    }
}

final class Selection: UITextSelectionRect {
    private let r: CGRect
    init(_ r: CGRect) { self.r = r }
    override var rect: CGRect { r }
    override var writingDirection: NSWritingDirection { .natural }
    override var containsStart: Bool { true }
    override var containsEnd: Bool { true }
    override var isVertical: Bool { false }
}
