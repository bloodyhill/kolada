//! Reminders of what is due (`kolada-app` `model/agenda.rs`), shown by the
//! Mac's Notification Center. The first one asks the person whether Kolada
//! may notify (macOS's own question); if not, nothing shows. They show
//! while Kolada is in front too, as Windows' do. Only an app bundle has
//! notifications: a bare binary (tests, `cargo run`) shows none.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{AnyThread, ClassType, class, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSString};

/// The framework, loaded at the first reminder rather than linked: linking
/// it costs every launch its load before the first frame.
fn load() -> bool {
    static LOADED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOADED.get_or_init(|| {
        let path = c"/System/Library/Frameworks/UserNotifications.framework/UserNotifications";
        !unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY) }.is_null() && objc2::runtime::AnyClass::get(c"UNUserNotificationCenter").is_some()
    })
}

/// Alert and sound (`UNAuthorizationOptions`).
const ASK: usize = (1 << 1) | (1 << 2);
/// Banner, list and sound (`UNNotificationPresentationOptions`).
const PRESENT: usize = (1 << 1) | (1 << 3) | (1 << 4);

/// The Notification Center, for the model's `Notify` service.
pub struct MacNotify;

define_class!(
    /// Says to show a reminder even while Kolada is in front.
    #[unsafe(super(NSObject))]
    #[name = "KoladaNotifyDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &AnyObject, _note: &AnyObject, done: &block2::DynBlock<dyn Fn(usize)>) {
            done.call((PRESENT,));
        }
    }
);

/// Whether this process is an app bundle (only those may notify).
fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some() && NSBundle::mainBundle().bundlePath().to_string().ends_with(".app")
}

fn center() -> Retained<AnyObject> {
    unsafe { msg_send![class!(UNUserNotificationCenter), currentNotificationCenter] }
}

/// The notification itself, once allowed.
fn post(title: &str, text: &str) {
    unsafe {
        let content: Retained<AnyObject> = msg_send![class!(UNMutableNotificationContent), new];
        let _: () = msg_send![&*content, setTitle: &*NSString::from_str(title)];
        let _: () = msg_send![&*content, setBody: &*NSString::from_str(text)];
        let id = NSString::from_str(&format!("kolada-{}", kolada_core::session::now()));
        let request: Retained<AnyObject> = msg_send![class!(UNNotificationRequest), requestWithIdentifier: &*id, content: &*content, trigger: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![&*center(), addNotificationRequest: &*request, withCompletionHandler: std::ptr::null::<AnyObject>()];
    }
}

impl kolada_app::services::Notify for MacNotify {
    fn notify(&self, title: &str, text: &str) {
        if !bundled() || !load() {
            return;
        }
        thread_local! {
            static DELEGATE: std::cell::OnceCell<Retained<Delegate>> = const { std::cell::OnceCell::new() };
        }
        let center = center();
        DELEGATE.with(|d| {
            let delegate = d.get_or_init(|| unsafe { msg_send![Delegate::alloc(), init] });
            let delegate: &AnyObject = delegate.as_super().as_ref();
            let _: () = unsafe { msg_send![&*center, setDelegate: delegate] };
        });
        let (title, text) = (title.to_string(), text.to_string());
        // Asked once; later calls answer at once with what was chosen.
        let asked = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            if granted.as_bool() {
                post(&title, &text);
            }
        });
        let _: () = unsafe { msg_send![&*center, requestAuthorizationWithOptions: ASK, completionHandler: &*asked] };
    }
}
