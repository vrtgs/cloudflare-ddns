use std::borrow::Cow;
use std::panic::PanicHookInfo;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::sync::Semaphore;

pub mod exit;

#[macro_export]
macro_rules! dbg_println {
    ($($arg:tt)*) => {
        { #[cfg(debug_assertions)] { eprintln!($($arg)*) } }
    };
}

fn spawn_thread(fun: impl FnOnce() + Send + 'static) {
    let handle = Handle::try_current();
    match handle {
        Ok(handle) => {
            handle.spawn_blocking(fun);
        }
        Err(_) => {
            thread::spawn(fun);
        }
    }
}

#[macro_export]
macro_rules! abort {
    ($($args:tt)*) => {{
        use ::std::borrow::Cow;
        let msg = ::std::format_args!($($args)*)
            .as_str()
            .map_or_else(
                || Cow::Owned(::std::format!($($args)*)),
                Cow::Borrowed
            );

        $crate::err::error(&msg);
        ::std::process::abort()
    }};
}

#[macro_export]
macro_rules! abort_unreachable {
    ($($args:tt)*) => {
        $crate::abort!("UNREACHABLE CONDITION: {}\n\n\n\naborting...", ::std::format_args!($($args)*))
    };
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsStr;
    use std::num::NonZeroU16;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w as wide, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MESSAGEBOX_STYLE,
    };

    fn encode_wide(str: &OsStr) -> Vec<u16> {
        str.encode_wide()
            .filter_map(NonZeroU16::new)
            .map(NonZeroU16::get)
            .chain([0u16])
            .collect::<Vec<u16>>()
    }

    /// # Safety:
    ///   `caption`: has to be a valid, aligned pointer to a constant null-terminated string of 16-bit Unicode characters.
    unsafe fn present_alert(caption: PCWSTR, msg: &OsStr, style: MESSAGEBOX_STYLE) {
        let msg = encode_wide(msg);
        unsafe {
            MessageBoxW(None, PCWSTR::from_raw(msg.as_ptr()), caption, MB_OK | style);
        }
    }

    pub fn warn(warning: &str) {
        // # Safety: caption was made by the wide macro which is valid
        unsafe {
            present_alert(
                wide!("CloudFlare DDNS Warning"),
                warning.as_ref(),
                MB_ICONWARNING,
            )
        }
    }

    pub fn err(err: &str) {
        // # Safety: caption was made by the wide macro which is valid
        unsafe { present_alert(wide!("CloudFlare DDNS Error"), err.as_ref(), MB_ICONERROR) }
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::CFOptionFlags;
    use core_foundation_sys::user_notification::{
        kCFUserNotificationCautionAlertLevel, kCFUserNotificationStopAlertLevel,
        CFUserNotificationDisplayAlert,
    };

    fn present_alert(title: &str, message: &str, flags: CFOptionFlags) {
        let header = CFString::new(title);
        let message = CFString::new(message);
        unsafe {
            CFUserNotificationDisplayAlert(
                0.0,
                flags,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                header.as_concrete_TypeRef(),
                message.as_concrete_TypeRef(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
    }

    pub fn warn(warning: &str) {
        present_alert(
            "CloudFlare DDNS Warning",
            warning,
            kCFUserNotificationCautionAlertLevel,
        );
    }

    pub fn err(err: &str) {
        present_alert(
            "CloudFlare DDNS Error",
            err,
            kCFUserNotificationStopAlertLevel,
        );
    }
}

#[cfg(target_os = "linux")]
mod sys {
    // use gtk4::prelude::*;
    // use gtk4::{Application, ApplicationWindow, ButtonsType, MessageDialog, MessageType};
    use log::{Log, Metadata, Record};
    use simple_logger::SimpleLogger;
    use std::ops::Deref;
    use std::sync::OnceLock;
    use systemd_journal_logger::JournalLog;

    enum OneOrTwo<T> {
        One(T),
        Two([T; 2]),
    }

    struct Loggers(OneOrTwo<Box<dyn Log>>);

    impl Loggers {
        fn iter(&self) -> impl Iterator<Item = &dyn Log> {
            match &self.0 {
                OneOrTwo::One(one) => std::slice::from_ref(one),
                OneOrTwo::Two(two) => two,
            }
            .iter()
            .map(Deref::deref)
        }
    }

    impl Default for Loggers {
        fn default() -> Self {
            let simple = Box::new(SimpleLogger::new()) as Box<dyn Log>;
            let system_d = JournalLog::new().map(Box::new);
            let stuff = match system_d {
                Ok(system_d) => OneOrTwo::Two([simple, system_d]),
                Err(_) => OneOrTwo::One(simple),
            };
            Self(stuff)
        }
    }

    impl Log for Loggers {
        fn enabled(&self, metadata: &Metadata) -> bool {
            self.iter().any(|logger| logger.enabled(metadata))
        }

        fn log(&self, record: &Record) {
            self.iter().for_each(|logger| logger.log(record))
        }

        fn flush(&self) {
            self.iter().for_each(Log::flush)
        }
    }

    static LOGGERS: OnceLock<Loggers> = OnceLock::new();

    static GTK_AVAILABLE: OnceLock<ErrorBackEnd> = OnceLock::new();

    // type GtkMessage = (Box<dyn FnOnce() + Send>, OneShotSender<()>);
    #[derive(Clone)]
    enum ErrorBackEnd {
        // Gtk(std::sync::mpsc::Sender<GtkMessage>),
        Logger,
    }

    fn back_end() -> ErrorBackEnd {
        GTK_AVAILABLE
            .get_or_init(|| {
                // let (send, rcv) = std::sync::mpsc::sync_channel(1);
                // let _ = thread::Builder::new().spawn(move || {
                //     if gtk4::init().is_err() {
                //         let _ = send.send(Err(()));
                //         return;
                //     }
                //     let (task_send, rcv) = std::sync::mpsc::channel::<GtkMessage>();
                //     let _ = send.send(Ok(task_send));
                //
                //     for (msg, cb) in rcv {
                //         msg();
                //         let _ = cb.send(());
                //     }
                // });
                //
                // match rcv.recv() {
                //     Ok(Ok(sender)) => ErrorBackEnd::Gtk(sender),
                //     _ => {
                //         log::set_logger(LOGGERS.get_or_init(Loggers::default))
                //             .expect("unable to set any form of logging");
                //         ErrorBackEnd::Logger
                //     }
                // }

                log::set_logger(LOGGERS.get_or_init(Loggers::default))
                    .expect("unable to set any form of logging");
                ErrorBackEnd::Logger
            })
            .clone()
    }

    // fn gtk_present(title: String, msg: String, message_type: MessageType) {
    //     let app = Application::builder()
    //         .application_id("xyz.vrtgs.cloudflare-ddns.errors")
    //         .build();
    //
    //     app.connect_activate(move |app| {
    //         let window = ApplicationWindow::builder()
    //             .application(app)
    //             .title(&title)
    //             .default_width(0)
    //             .default_height(0)
    //             .build();
    //
    //         let dialog = MessageDialog::builder()
    //             .transient_for(&window)
    //             .modal(true)
    //             .buttons(ButtonsType::Close)
    //             .message_type(message_type)
    //             .text(&title)
    //             .secondary_text(&msg)
    //             .build();
    //
    //         let app = app.clone();
    //         dialog.connect_response(move |dialog, _| {
    //             dialog.close();
    //             window.close();
    //             app.quit();
    //         });
    //
    //         dialog.show();
    //     });
    //
    //     app.run();
    // }

    fn present_alert(title: &str, msg: &str, message_type: log::Level) {
        match back_end() {
            // ErrorBackEnd::Gtk(chan) => {
            //     let (title, msg) = (title.to_owned(), msg.to_owned());
            //     let (send, wait) = std::sync::mpsc::sync_channel(1);
            //     let _ = chan.send((
            //         Box::new(move || gtk_present(title, msg, message_type)),
            //         send,
            //     ));
            //     let _ = wait.recv();
            // }
            ErrorBackEnd::Logger => match message_type {
                log::Level::Warn => log::warn!("[{title}]: {msg}"),
                log::Level::Error => log::error!("[{title}]: {msg}"),
                _ => unreachable!(),
            },
        }
    }

    pub fn warn(warning: &str) {
        present_alert("CloudFlare DDNS Warning", warning, log::Level::Warn);
    }

    pub fn err(err: &str) {
        present_alert("CloudFlare DDNS Error", err, log::Level::Error);
    }
}

#[cold]
#[inline(never)]
pub fn error(err: &str) {
    dbg_println!("Error: {err}");
    sys::err(err)
}

#[cold]
#[inline(never)]
pub fn warn(warning: &str) {
    dbg_println!("Warning: {warning}");
    sys::warn(warning)
}

pub async fn spawn_message_box(semaphore: Arc<Semaphore>, err: impl FnOnce() + Send + 'static) {
    if let Ok(permit) = semaphore.acquire_owned().await {
        spawn_thread(move || {
            err();
            drop(permit);
        });
    }
}

fn hook(info: &PanicHookInfo) {
    macro_rules! try_cast {
        ([$payload:expr] $type: ty $(, $rest: ty)* |> $default: expr) => {
            match $payload.downcast_ref::<$type>() {
                Some(s) => s,
                None => try_cast!([$payload] $($rest),* |> $default),
            }
        };
        ([$_:expr] |> $default: expr) => { $default };
    }

    let msg = try_cast!([info.payload()] String,&str,Box<str>,Rc<str>,Arc<str>,Cow<str> |> "Box<dyn Any>");

    dbg_println!("We panicked at: {msg}");

    match Handle::try_current().as_ref().map(Handle::runtime_flavor) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(|| error(msg)),
        _ => error(msg),
    }
}

pub fn set_hook() {
    std::panic::set_hook(Box::new(hook));
}
