//! Deleting this computer's account at its server: everything the server keeps of it, for every
//! device (`remote_proto::delete`). The desktop app asks from Settings → Remote, after its person
//! typed the confirmation; then they sign in again in the browser, and that is what deletes. A
//! person's act, never an agent's: nothing an agent is given reaches it.
//!
//! Everything on this computer stays: projects, workspaces, agents and their terminals. Only its
//! sign-in goes, and Remote is off here afterwards.

use remote_client::{Client, SignIn};
use remote_proto::delete::{AccountDeleted, DELETE_CAPABILITY};
use remote_proto::ErrorCode;

use crate::account::{self, client, enabled, failed, refused_here, Failure, Home, Result};

/// Start the sign-in again that confirms the deletion: the address to open in the browser is
/// [`SignIn::url`]. Refused before anything is asked of the person when the server cannot delete.
pub async fn begin(home: &impl Home) -> Result<(Client, SignIn)> {
    let row = enabled(home)?;
    let client = Client::new(&row.server_url).map_err(failed)?;
    let server = client.describe().await.map_err(failed)?;
    if !server.capabilities.iter().any(|c| c == DELETE_CAPABILITY) {
        return Err(Failure::new(
            "remote_refused",
            "This server cannot delete an account yet. Its operator has to update it.",
        ));
    }
    let sign_in = client.begin_confirm().await.map_err(failed)?;
    Ok((client, sign_in))
}

/// The browser came back with `code`: delete the account, then forget it here.
pub async fn finish(home: &impl Home, sign_in: SignIn, code: String) -> Result<AccountDeleted> {
    let row = enabled(home)?;
    let (client, access) = client(home, &row).await?;
    let deleted = sign_in
        .delete_account(&client, &access, code)
        .await
        .map_err(|error| match error.code() {
            // Signed in again as someone else: the server's words say so.
            Some(ErrorCode::Forbidden) => Failure::new("remote_other_account", error.to_string()),
            // A confirmation that lapsed or was spent is the server's to explain, too; only
            // this computer's own credential being refused is "signed out".
            Some(ErrorCode::Unauthenticated) if !error.account_deleted() => {
                Failure::new("remote_not_confirmed", error.to_string())
            }
            _ => refused_here(error),
        })?;
    account::forget(home, &row).await?;
    Ok(deleted)
}
