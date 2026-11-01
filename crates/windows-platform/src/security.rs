use windows_sys::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WinBuiltinAdministratorsSid,
};

/// True when this process has the rights of the administrators. Without the UAC prompt, an
/// administrator has the group only to deny access, and this is false.
pub fn is_administrator() -> bool {
    let mut size = SECURITY_MAX_SID_SIZE;
    // A buffer of `u32` has the alignment that a SID needs.
    let mut administrators = vec![0u32; (SECURITY_MAX_SID_SIZE as usize).div_ceil(4)];

    let created = unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            std::ptr::null_mut(),
            administrators.as_mut_ptr().cast(),
            &raw mut size,
        )
    };

    if created == 0 {
        return false;
    }

    let mut member = 0;

    // A null token is the token of this thread or process.
    let checked = unsafe {
        CheckTokenMembership(
            std::ptr::null_mut(),
            administrators.as_mut_ptr().cast(),
            &raw mut member,
        )
    };

    checked != 0 && member != 0
}
