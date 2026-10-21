// Notes going out (the share sheet: Markdown, a web page, a PDF printed
// from it) and files coming in by drag and drop (crates/kolada-ios's
// arrive.rs: the card says what each becomes as it comes over the note,
// and pictures land in the gap it opens).

import UIKit
import UniformTypeIdentifiers

extension NoteView {
    /// The note sent out through the share sheet (Save to Files, AirDrop,
    /// Mail...) as Markdown, a web page or a PDF printed from it; `choose`
    /// asks which first.
    func export(_ kind: String, name: String) {
        if kind == "choose" {
            let sheet = UIAlertController(title: "Export note", message: nil, preferredStyle: .actionSheet)
            for (title, k) in [("Markdown", "markdown"), ("PDF", "pdf"), ("Web page", "html")] {
                sheet.addAction(UIAlertAction(title: title, style: .default) { [weak self] _ in self?.export(k, name: name) })
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = self
            sheet.popoverPresentationController?.sourceRect = CGRect(x: bounds.maxX - 60, y: 20, width: 1, height: 1)
            window?.rootViewController?.present(sheet, animated: true)
            return
        }
        guard let page = kolada_export(app, kind) else { return }
        let text = String(cString: page)
        kolada_free_string(page)
        let ext = ["markdown": "md", "pdf": "pdf"][kind] ?? "html"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(name).\(ext)")
        do {
            if kind == "pdf" {
                try pdf(of: text).write(to: url)
            } else {
                try text.write(to: url, atomically: true, encoding: .utf8)
            }
        } catch {
            return
        }
        let share = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        share.popoverPresentationController?.sourceView = self
        share.popoverPresentationController?.sourceRect = CGRect(x: bounds.maxX - 60, y: 20, width: 1, height: 1)
        window?.rootViewController?.present(share, animated: true)
    }

    /// A web page printed to A4 pages (595 x 842 points) with margins.
    private func pdf(of html: String) -> Data {
        let renderer = UIPrintPageRenderer()
        renderer.addPrintFormatter(UIMarkupTextPrintFormatter(markupText: html), startingAtPageAt: 0)
        let paper = CGRect(x: 0, y: 0, width: 595, height: 842)
        renderer.setValue(paper, forKey: "paperRect")
        renderer.setValue(paper.insetBy(dx: 48, dy: 56), forKey: "printableRect")
        let data = NSMutableData()
        UIGraphicsBeginPDFContextToData(data, paper, nil)
        for page in 0..<max(renderer.numberOfPages, 1) {
            UIGraphicsBeginPDFPage()
            renderer.drawPage(at: page, in: UIGraphicsGetPDFContextBounds())
        }
        UIGraphicsEndPDFContext()
        return data as Data
    }
}

extension NoteView: UIDropInteractionDelegate {
    /// A dragged item's file name, as iOS says it before its contents come.
    private static func name(of item: UIDragItem) -> String {
        let provider = item.itemProvider
        let type = provider.registeredTypeIdentifiers.compactMap(UTType.init).first { $0.conforms(to: .data) || $0.conforms(to: .item) }
        let base = provider.suggestedName ?? "Untitled"
        guard let ext = type?.preferredFilenameExtension, !base.lowercased().hasSuffix("." + ext) else { return base }
        return base + "." + ext
    }

    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
        // Text dragged within Kolada is the text system's own, not a file.
        session.localDragSession == nil && session.hasItemsConforming(toTypeIdentifiers: [UTType.data.identifier])
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidEnter session: UIDropSession) {
        let p = session.location(in: self)
        _ = kolada_drag_enter(app, session.items.map(Self.name).joined(separator: "\n"), Float(p.x), Float(p.y))
        redraw()
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal {
        let p = session.location(in: self)
        let takes = kolada_drag_over(app, Float(p.x), Float(p.y))
        redraw()
        return UIDropProposal(operation: takes ? .copy : .forbidden)
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidExit session: UIDropSession) {
        _ = kolada_drag_leave(app)
        redraw()
    }

    func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        let p = session.location(in: self)
        let end = kolada_drop(app)
        redraw()
        // iOS hands each file over for a moment: copied somewhere of the app's own first.
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("Dropped-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let group = DispatchGroup()
        let lock = NSLock()
        var paths: [String] = []
        for item in session.items {
            guard let type = item.itemProvider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .data) ?? false }) else { continue }
            let name = Self.name(of: item)
            group.enter()
            item.itemProvider.loadFileRepresentation(forTypeIdentifier: type) { url, _ in
                defer { group.leave() }
                guard let url else { return }
                let to = folder.appendingPathComponent(name)
                if (try? FileManager.default.copyItem(at: url, to: to)) != nil {
                    lock.lock()
                    paths.append(to.path)
                    lock.unlock()
                }
            }
        }
        group.notify(queue: .main) { [weak self] in
            guard let self, !paths.isEmpty else { return }
            self.outside { kolada_drop_files(self.app, paths.joined(separator: "\n"), Float(p.x), Float(p.y), end) }
        }
    }
}
