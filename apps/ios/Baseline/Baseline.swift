// The floor for the iPad's launch promise: an empty UIKit app, scene and
// view as Kolada's, that marks when its first frame is on its layer. Built
// and launched by tools/ios-launch.sh beside Kolada, alternately.

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
        let controller = UIViewController()
        controller.view = Empty()
        window.rootViewController = controller
        window.makeKeyAndVisible()
        self.window = window
    }
}

final class Empty: UIView {
    private var presented = false
    override func layoutSubviews() {
        super.layoutSubviews()
        guard !presented, bounds.width > 0 else { return }
        presented = true
        // One plain frame, as small a cost as a frame can have.
        layer.backgroundColor = UIColor.systemBackground.cgColor
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.size
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return }
        let start = info.kp_proc.p_un.__p_starttime
        var now = timeval()
        gettimeofday(&now, nil)
        let ms = Double(now.tv_sec - start.tv_sec) * 1000 + Double(now.tv_usec - start.tv_usec) / 1000
        FileHandle.standardError.write("first frame presented: \(String(format: "%.1f", ms)) ms\n".data(using: .utf8)!)
    }
}
