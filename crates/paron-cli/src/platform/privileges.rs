pub fn is_user_root() -> bool {
    #[cfg(windows)]
    {
        windows_platform::security::is_administrator()
    }

    #[cfg(unix)]
    {
        rustix::process::geteuid().is_root()
    }
}
