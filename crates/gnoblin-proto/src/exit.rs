/// The exit status the compositor uses when it stops on a user's logout request.
///
/// After a logout is confirmed the compositor flushes its state and exits with
/// this code. The session guardian reads it as a deliberate end of the session
/// rather than a crash, so it stops the other children in order and does not
/// restart anything. Any other non-zero exit is still a lost compositor.
pub const LOGOUT_EXIT_CODE: u8 = 43;
