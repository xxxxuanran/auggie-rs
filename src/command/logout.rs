use anyhow::Result;

use crate::session::AuthSessionStore;

pub async fn run_logout() -> Result<()> {
    let session_store = AuthSessionStore::new(None)?;

    if !session_store.is_logged_in() {
        println!("You are not logged in.");
        return Ok(());
    }

    session_store.remove_session()?;
    println!("✅ Successfully logged out from Augment.");

    Ok(())
}
