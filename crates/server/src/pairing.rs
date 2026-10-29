//! Hosts, the offers they show, and the grants their people give: see [`remote_proto::pairing`].
//!
//! Every check here is the caller's account first. Another account's host, offer or grant is
//! `not_found`, exactly as one that does not exist. Within the account, a host's offers and
//! answers come from the device the host is signed in as now, and nobody else; a grant is seen
//! by its host and by the device it is for.

use remote_proto::account::DeviceKind;
use remote_proto::pairing::{
    normalize_code, GrantDecision, GrantPage, GrantState, GrantSummary, HostPage, HostRegistration,
    HostSummary, Offer, OfferKey, PairingRequest, OFFER_TTL_MS,
};
use remote_proto::{ErrorCode, GrantId, HostId, RemoteError};

use crate::records::{
    Decision, GrantFilter, GrantRecord, GrantStatus, GrantView, Host, HostOffer, Records,
    Redemption,
};
use crate::service::unreadable;
use crate::tokens::{self, Kind};
use crate::{Caller, ControlService};

/// The most grants one list returns, newest first.
pub const MAX_LISTED_GRANTS: usize = 100;

impl<R: Records> ControlService<R> {
    /// `POST /v1/hosts`: a host signs in and registers, or registers again under the id this
    /// server gave it before.
    pub async fn register_host(
        &self,
        caller: &Caller,
        registration: &HostRegistration,
        now: i64,
    ) -> Result<HostSummary, RemoteError> {
        registration.validate()?;
        if caller.kind != DeviceKind::Host {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "Only a device signed in as a host can register as one.",
            ));
        }
        let name = registration.name.trim();
        let host = match &registration.host {
            Some(id) => self
                .records
                .rebind_host(&caller.account, id, &caller.device, name)
                .await
                .map_err(unreadable)?
                .ok_or_else(RemoteError::not_found)?,
            None => {
                let host = Host {
                    id: HostId::new(uuid::Uuid::new_v4().to_string()).map_err(unreadable)?,
                    account: caller.account.clone(),
                    device: caller.device.clone(),
                    name: name.to_owned(),
                    created_at: now,
                    last_seen_at: None,
                };
                self.records
                    .add_host(host.clone())
                    .await
                    .map_err(unreadable)?;
                host
            }
        };
        Ok(host_summary(host, caller))
    }

    /// `GET /v1/hosts`: the account's hosts, newest first.
    pub async fn hosts(&self, caller: &Caller) -> Result<HostPage, RemoteError> {
        let mut hosts = self
            .records
            .hosts(&caller.account)
            .await
            .map_err(unreadable)?;
        hosts.sort_by_key(|h| std::cmp::Reverse(h.created_at));
        Ok(HostPage {
            hosts: hosts.into_iter().map(|h| host_summary(h, caller)).collect(),
        })
    }

    /// `POST /v1/hosts/{host}/offers`: a new offer, in place of any the host still had open.
    pub async fn offer(
        &self,
        caller: &Caller,
        host: &HostId,
        now: i64,
    ) -> Result<Offer, RemoteError> {
        let host = self.own_host(caller, host).await?;
        let secret = tokens::mint(Kind::Offer);
        let code = tokens::short_code();
        self.records
            .put_offer(
                HostOffer {
                    host: host.id,
                    account: caller.account.clone(),
                    secret: secret.hash,
                    code: code.hash,
                    expires_at: now + OFFER_TTL_MS,
                },
                now,
            )
            .await
            .map_err(unreadable)?;
        Ok(Offer {
            secret: secret.secret,
            code: code.secret,
            expires_in_ms: OFFER_TTL_MS as u64,
        })
    }

    /// `POST /v1/pairings`: the caller redeems an offer of its own account and asks its host
    /// for a grant. This asks; nothing is given until the host's person approves.
    pub async fn pair(
        &self,
        caller: &Caller,
        request: &PairingRequest,
        now: i64,
    ) -> Result<GrantSummary, RemoteError> {
        request.validate()?;
        let wrong = || {
            RemoteError::new(
                ErrorCode::NotFound,
                "That pairing code is wrong or has expired. Ask the host for a new one.",
            )
        };
        let key = match &request.offer {
            OfferKey::Secret(secret) => {
                tokens::presented(Kind::Offer, secret.expose()).ok_or_else(wrong)?
            }
            OfferKey::Code(code) => {
                tokens::code_hash(&normalize_code(code.expose()).ok_or_else(wrong)?)
            }
        };
        let offer = match self
            .records
            .redeem_offer(&caller.account, &key, now)
            .await
            .map_err(unreadable)?
        {
            Redemption::Taken(offer) => offer,
            Redemption::Missed => return Err(wrong()),
        };
        let host = self
            .records
            .host(&caller.account, &offer.host)
            .await
            .map_err(unreadable)?
            .ok_or_else(wrong)?;
        if host.device == caller.device {
            return Err(RemoteError::new(
                ErrorCode::InvalidRequest,
                "A host cannot pair with itself. Use the code on another device.",
            ));
        }
        let grant = GrantRecord {
            id: GrantId::new(uuid::Uuid::new_v4().to_string()).map_err(unreadable)?,
            account: caller.account.clone(),
            host: host.id,
            device: caller.device.clone(),
            requested: request.scopes.clone(),
            scopes: Default::default(),
            projects: None,
            status: GrantStatus::Pending,
            created_at: now,
            expires_at: now + OFFER_TTL_MS,
            decided_at: None,
        };
        self.records
            .add_grant(grant.clone())
            .await
            .map_err(unreadable)?;
        self.visible_grant(caller, &grant.id, now).await
    }

    /// `GET /v1/hosts/{host}/grants`: for the host, the grants waiting for its person and the
    /// ones in force. Anything else is no longer the host's concern.
    pub async fn host_grants(
        &self,
        caller: &Caller,
        host: &HostId,
        now: i64,
    ) -> Result<GrantPage, RemoteError> {
        let host = self.own_host(caller, host).await?;
        let views = self
            .records
            .grants(&caller.account, GrantFilter::OfHost(host.id))
            .await
            .map_err(unreadable)?;
        Ok(GrantPage {
            grants: views
                .into_iter()
                .map(|v| grant_summary(v, now))
                .filter(|g| matches!(g.state, GrantState::Pending | GrantState::Approved))
                .take(MAX_LISTED_GRANTS)
                .collect(),
        })
    }

    /// `GET /v1/grants`: the caller's own grants and requests, newest first.
    pub async fn device_grants(&self, caller: &Caller, now: i64) -> Result<GrantPage, RemoteError> {
        let views = self
            .records
            .grants(
                &caller.account,
                GrantFilter::ForDevice(caller.device.clone()),
            )
            .await
            .map_err(unreadable)?;
        Ok(GrantPage {
            grants: views
                .into_iter()
                .take(MAX_LISTED_GRANTS)
                .map(|v| grant_summary(v, now))
                .collect(),
        })
    }

    /// `GET /v1/grants/{grant}`: for the device it is for, and for its host.
    pub async fn grant(
        &self,
        caller: &Caller,
        id: &GrantId,
        now: i64,
    ) -> Result<GrantSummary, RemoteError> {
        self.visible_grant(caller, id, now).await
    }

    /// `POST /v1/grants/{grant}/approve` with a decision, or `…/deny` without one. Only the host
    /// answers, and only while the grant waits.
    pub async fn decide(
        &self,
        caller: &Caller,
        id: &GrantId,
        decision: Option<&GrantDecision>,
        now: i64,
    ) -> Result<GrantSummary, RemoteError> {
        let view = self.grant_view(caller, id).await?;
        self.own_host(caller, &view.grant.host).await?;
        let not_waiting = || {
            RemoteError::new(
                ErrorCode::InvalidRequest,
                "This request is no longer waiting for an answer.",
            )
        };
        if grant_summary(view.clone(), now).state != GrantState::Pending {
            return Err(not_waiting());
        }
        let decision = match decision {
            Some(decision) => {
                decision.validate(&view.grant.requested)?;
                Decision::Approve {
                    scopes: decision.scopes.clone(),
                    projects: decision.projects.clone(),
                }
            }
            None => Decision::Deny,
        };
        self.records
            .decide(&caller.account, id, decision, now)
            .await
            .map_err(unreadable)?
            .ok_or_else(not_waiting)?;
        // An approval revokes the device's earlier grant on this host.
        self.relay.nudge(&caller.account);
        self.visible_grant(caller, id, now).await
    }

    /// `POST /v1/grants/{grant}/revoke`: by its host or by the device it is for.
    pub async fn revoke_grant(
        &self,
        caller: &Caller,
        id: &GrantId,
        now: i64,
    ) -> Result<(), RemoteError> {
        self.grant_view(caller, id).await?;
        self.records
            .revoke_grant(&caller.account, id, now)
            .await
            .map_err(unreadable)?;
        self.relay.nudge(&caller.account);
        Ok(())
    }

    /// `POST /v1/hosts/{host}/remove`: the host stops being one. Its offers go and its grants are
    /// revoked. Only the host itself does this; another device signs it out instead.
    pub async fn remove_host(
        &self,
        caller: &Caller,
        host: &HostId,
        now: i64,
    ) -> Result<(), RemoteError> {
        let host = self.own_host(caller, host).await?;
        self.records
            .remove_host(&caller.account, &host.id, now)
            .await
            .map_err(unreadable)?;
        self.relay.nudge(&caller.account);
        Ok(())
    }

    /// The account's host, if the caller is the device it is signed in as now.
    async fn own_host(&self, caller: &Caller, id: &HostId) -> Result<Host, RemoteError> {
        self.records
            .host(&caller.account, id)
            .await
            .map_err(unreadable)?
            .filter(|h| h.device == caller.device)
            .ok_or_else(RemoteError::not_found)
    }

    /// A grant the caller may see: its own, or one on a host it is.
    async fn grant_view(&self, caller: &Caller, id: &GrantId) -> Result<GrantView, RemoteError> {
        let view = self
            .records
            .grants(&caller.account, GrantFilter::One(id.clone()))
            .await
            .map_err(unreadable)?
            .into_iter()
            .next()
            .ok_or_else(RemoteError::not_found)?;
        if view.grant.device == caller.device {
            return Ok(view);
        }
        let host = self
            .records
            .host(&caller.account, &view.grant.host)
            .await
            .map_err(unreadable)?;
        match host {
            Some(host) if host.device == caller.device => Ok(view),
            _ => Err(RemoteError::not_found()),
        }
    }

    async fn visible_grant(
        &self,
        caller: &Caller,
        id: &GrantId,
        now: i64,
    ) -> Result<GrantSummary, RemoteError> {
        Ok(grant_summary(self.grant_view(caller, id).await?, now))
    }
}

fn host_summary(host: Host, caller: &Caller) -> HostSummary {
    HostSummary {
        current: host.device == caller.device,
        id: host.id,
        name: host.name,
        created_at: host.created_at,
        last_seen_at: host.last_seen_at,
    }
}

/// A grant as the wire shows it. "Expired" and "revoked by signing its device out" are worked
/// out here, at `now`, rather than kept.
fn grant_summary(view: GrantView, now: i64) -> GrantSummary {
    let grant = view.grant;
    let state = match grant.status {
        GrantStatus::Pending | GrantStatus::Approved if view.device_revoked => GrantState::Revoked,
        GrantStatus::Pending if grant.expires_at <= now => GrantState::Expired,
        GrantStatus::Pending => GrantState::Pending,
        GrantStatus::Approved => GrantState::Approved,
        GrantStatus::Denied => GrantState::Denied,
        GrantStatus::Revoked => GrantState::Revoked,
    };
    GrantSummary {
        id: grant.id,
        host: grant.host,
        host_name: view.host_name,
        device: grant.device,
        device_name: view.device_name,
        state,
        requested: grant.requested,
        scopes: grant.scopes,
        projects: grant.projects,
        created_at: grant.created_at,
        decided_at: grant.decided_at,
        expires_in_ms: (state == GrantState::Pending)
            .then(|| u64::try_from(grant.expires_at - now).unwrap_or_default()),
    }
}
