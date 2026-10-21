// The model's menus (crates/kolada-ios/src/menus.rs), shown in the system's
// edit menu: over the text after a long press or a double tap (a misspelled
// word's fixes first, then the pasteboard, formatting and the note), at once
// on a row of the side menu, under a row's three dots and the send button.

import UIKit

extension NoteView: UIEditMenuInteractionDelegate {
    /// The menu for what is at `p`.
    func showMenu(at p: CGPoint) {
        editMenu.presentEditMenu(with: UIEditMenuConfiguration(identifier: "at" as NSString, sourcePoint: p))
    }

    /// The menu the model opened from one of its buttons, at its point.
    func showRequestedMenu(at p: CGPoint) {
        editMenu.presentEditMenu(with: UIEditMenuConfiguration(identifier: "requested" as NSString, sourcePoint: p))
    }

    func editMenuInteraction(_ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration, suggestedActions: [UIMenuElement]) -> UIMenu? {
        let p = configuration.sourcePoint
        let lines: UnsafeMutablePointer<CChar>?
        if (configuration.identifier as? NSString) == "requested" {
            lines = kolada_menu_requested(app)
        } else {
            let board = UIPasteboard.general
            lines = kolada_menu(app, model(p).x, model(p).y, board.contains(pasteboardTypes: ["public.html"]))
            // The caret may have gone to where the press was.
            redraw()
        }
        guard let lines else { return nil }
        defer { kolada_free_string(lines) }
        // Groups, as the model's dividers say.
        var groups: [[UIMenuElement]] = [[]]
        for (i, line) in String(cString: lines).split(separator: "\n", omittingEmptySubsequences: false).enumerated() where !line.isEmpty {
            let f = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
            if f.count > 2, f[2] == "1", !(groups.last?.isEmpty ?? true) { groups.append([]) }
            groups[groups.count - 1].append(UIAction(title: f[0]) { [weak self] _ in
                guard let self else { return }
                self.outside { kolada_menu_pick(self.app, i) }
            })
        }
        return UIMenu(children: groups.filter { !$0.isEmpty }.map { UIMenu(options: .displayInline, children: $0) })
    }
}
