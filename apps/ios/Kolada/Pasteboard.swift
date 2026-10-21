// The pasteboard (crates/kolada-ios/src/clip.rs): a copy puts plain text,
// HTML and Kolada's own Markdown on it at once; a paste takes Kolada's own
// first (a copy within Kolada comes back exactly), then formatted text as
// Markdown, then plain text; a picture alone (a screenshot) is embedded.

import UIKit

extension NoteView {
    static let koladaMarkdown = "app.kolada.markdown"

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        switch action {
        case #selector(copy(_:)), #selector(cut(_:)):
            var location = 0, length = 0
            kolada_selection(app, &location, &length)
            return length > 0
        case #selector(paste(_:)), #selector(pastePlain(_:)):
            return UIPasteboard.general.hasStrings || UIPasteboard.general.hasImages
        case #selector(selectAll(_:)):
            return true
        default:
            return super.canPerformAction(action, withSender: sender)
        }
    }

    /// The three kinds of a copy (or cut, `take`).
    private func copied(take: Bool) {
        var plain: UnsafeMutablePointer<CChar>?, html: UnsafeMutablePointer<CChar>?, markdown: UnsafeMutablePointer<CChar>?
        if take {
            outside { kolada_cut(app, &plain, &html, &markdown) }
        } else {
            _ = kolada_copy(app, &plain, &html, &markdown)
        }
        defer { [plain, html, markdown].forEach { kolada_free_string($0) } }
        guard let plain else { return }
        var item: [String: Any] = ["public.utf8-plain-text": String(cString: plain)]
        if let html, html.pointee != 0 { item["public.html"] = String(cString: html) }
        if let markdown { item[Self.koladaMarkdown] = Data(String(cString: markdown).utf8) }
        UIPasteboard.general.items = [item]
    }

    override func copy(_ sender: Any?) { copied(take: false) }

    override func cut(_ sender: Any?) { copied(take: true) }

    override func paste(_ sender: Any?) {
        let board = UIPasteboard.general
        let markdown = board.data(forPasteboardType: Self.koladaMarkdown).flatMap { String(data: $0, encoding: .utf8) }
        let plain = board.string
        if plain == nil && markdown == nil, let png = board.image?.pngData() {
            outside { png.withUnsafeBytes { kolada_paste_picture(app, $0.bindMemory(to: UInt8.self).baseAddress, $0.count) } }
            return
        }
        let html = board.data(forPasteboardType: "public.html").flatMap { String(data: $0, encoding: .utf8) }
        outside { kolada_paste(app, plain ?? "", html, markdown) }
    }

    /// Cmd+Shift+V: the text alone, its formatting left behind.
    @objc func pastePlain(_ sender: Any?) {
        guard let plain = UIPasteboard.general.string else { return }
        outside { kolada_paste_plain(app, plain) }
    }

    override func selectAll(_ sender: Any?) {
        outside { kolada_command(app, "selectAll:") }
    }
}
