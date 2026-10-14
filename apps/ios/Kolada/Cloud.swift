// Vaults in iCloud Drive (Obsidian's iOS vaults live there too): notes iOS
// has not downloaded yet are hidden placeholders (`.Plan.md.icloud`), which
// the vault's listing skips like any hidden file. After the first frame,
// and each time the app comes to the front, the vault is looked through off
// the main thread, the placeholders are asked for, and the file tree is
// read again as they land. Never before the first frame: it is a walk of
// the whole vault.

import Foundation

enum Cloud {
    /// Placeholders in `vault`, as the files they stand for.
    private static func missing(in vault: URL) -> [URL] {
        guard let walk = FileManager.default.enumerator(at: vault, includingPropertiesForKeys: nil) else { return [] }
        var found: [URL] = []
        for case let url as URL in walk {
            let name = url.lastPathComponent
            guard name.hasPrefix("."), name.hasSuffix(".icloud") else { continue }
            found.append(url.deletingLastPathComponent().appendingPathComponent(String(name.dropFirst().dropLast(".icloud".count))))
        }
        return found
    }

    /// Asks iCloud for what `vault` is missing, then calls `arrived` on the
    /// main thread each time some of it lands (checked every 2 s for a
    /// minute), until it all has.
    static func fetch(_ vault: URL, arrived: @escaping () -> Void) {
        DispatchQueue.global(qos: .utility).async {
            var waiting = missing(in: vault)
            guard !waiting.isEmpty else { return }
            for url in waiting {
                try? FileManager.default.startDownloadingUbiquitousItem(at: url)
            }
            for _ in 0..<30 {
                Thread.sleep(forTimeInterval: 2)
                let still = waiting.filter { !FileManager.default.fileExists(atPath: $0.path) }
                if still.count < waiting.count {
                    DispatchQueue.main.async(execute: arrived)
                }
                waiting = still
                if waiting.isEmpty { return }
            }
        }
    }
}
