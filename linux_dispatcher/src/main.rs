cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        use std::os::unix::net::UnixStream;

        const SOCKET_PATH: &str =
            include_str!("../../cloudflare-ddns/src/network_listener/linux/socket-path");

        fn main() {
            let _ = UnixStream::connect(SOCKET_PATH);
        }
    } else {
        fn main() {
            panic!("available on linux only")
        }
    }
}
