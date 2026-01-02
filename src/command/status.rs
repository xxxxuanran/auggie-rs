use anyhow::Result;

use crate::session::AuthSessionStore;

pub async fn run_status() -> Result<()> {
    let session_store = AuthSessionStore::new(None)?;

    if session_store.is_logged_in() {
        if let Some(session) = session_store.get_session()? {
            println!("✅ Logged in to Augment");
            println!("   Tenant URL: {}", session.tenant_url);
            println!("   Scopes: {:?}", session.scopes);
        } else {
            println!("⚠️  Session file exists but is invalid.");
        }
    } else {
        println!("❌ Not logged in to Augment");
        println!("   Run 'auggie login' to authenticate.");
    }

    Ok(())
}
