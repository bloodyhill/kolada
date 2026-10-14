// Kolada for iPad and iPhone: the app and its window. Everything the app does
// is in Rust (crates/kolada-ios over crates/kolada-app); this file and
// NoteView.swift only connect UIKit to it.

import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession, options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: nil, sessionRole: session.role)
        config.delegateClass = SceneDelegate.self
        return config
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options connectionOptions: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: windowScene)
        window.rootViewController = NoteController()
        window.makeKeyAndVisible()
        self.window = window
    }

    // iOS suspends and ends apps without warning: the note is written as
    // soon as the app stops being in front, not only when it quits.
    func sceneWillResignActive(_ scene: UIScene) { note?.save() }
    func sceneDidEnterBackground(_ scene: UIScene) { note?.save() }
    func sceneDidDisconnect(_ scene: UIScene) { note?.save() }
    func sceneWillEnterForeground(_ scene: UIScene) { note?.cameForward() }

    private var note: NoteView? { (window?.rootViewController as? NoteController)?.note }
}

/// The window's one screen: the note view, under the status bar, above the
/// keyboard.
final class NoteController: UIViewController {
    let note = NoteView()

    override func loadView() {
        let root = UIView()
        root.addSubview(note)
        note.translatesAutoresizingMaskIntoConstraints = false
        let bottom = note.bottomAnchor.constraint(equalTo: root.keyboardLayoutGuide.topAnchor)
        NSLayoutConstraint.activate([
            note.topAnchor.constraint(equalTo: root.safeAreaLayoutGuide.topAnchor),
            note.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            note.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            bottom,
        ])
        root.keyboardLayoutGuide.followsUndockedKeyboard = true
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = note.paper
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        note.start()
    }

    override var prefersHomeIndicatorAutoHidden: Bool { true }
}
