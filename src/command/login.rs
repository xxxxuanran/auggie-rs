use anyhow::Result;

use crate::session::AuthSessionStore;
use crate::{api, oauth};

pub async fn run_login(login_url: Option<String>, augment_cache_dir: Option<String>) -> Result<()> {
    let login_url = login_url.unwrap_or_else(|| oauth::DEFAULT_AUTH_URL.to_string());

    let session_store = AuthSessionStore::new(augment_cache_dir.clone())?;

    // Check if already logged in
    if session_store.is_logged_in() {
        println!("⚠️  You are already logged in to Augment.");
        println!("Re-authenticating will replace your current session.\n");

        print!("Do you want to continue with re-authentication? This will invalidate your existing session. [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        let answer = answer.trim().to_lowercase();

        if answer != "y" && answer != "yes" {
            println!("Authentication cancelled. Your existing session remains active.");
            return Ok(());
        }

        println!("Removing existing session...");
        session_store.remove_session()?;
    }

    println!("🔐 Starting Augment authentication...\n");

    let api_client = api::ApiClient::new(None);
    let mut oauth_flow =
        oauth::OAuthFlow::new(&login_url, api_client, session_store, augment_cache_dir)?;

    // Start OAuth flow
    let authorize_url = oauth_flow.start_flow()?;

    // Ask user whether to open browser
    print!("Open authentication page in browser? [Y/n]: ");
    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    // Default to yes if user just presses Enter
    if answer.is_empty() || answer == "y" || answer == "yes" {
        println!("🌐 Opening authentication page in your browser...");
        if open::that(&authorize_url).is_err() {
            println!("⚠️  Could not open browser automatically.");
        }
    }

    println!("Please complete authentication in your browser:");
    println!("\n{}\n", authorize_url);
    println!("After authenticating, you will receive a JSON response.");
    println!("Copy the entire JSON response and paste it below.\n");

    print!("Paste the JSON response here: ");
    io::stdout().flush()?;

    let mut pasted = String::new();
    io::stdin().read_line(&mut pasted)?;
    let pasted = pasted.trim();

    oauth_flow.handle_auth_json(pasted).await?;

    println!("\n✅ Successfully authenticated with Augment!");

    Ok(())
}
