use crate::Result;

pub fn current_user() -> Result<String> {
    #[cfg(windows)]
    {
        super::windows::accounts::current_user_and_sid().map(|(name, _)| name)
    }

    #[cfg(unix)]
    {
        let uid = rustix::process::getuid().as_raw();

        super::unix::accounts::user_name(uid)?
            .ok_or_else(|| format!("could not get username of uid {uid}").into())
    }
}
