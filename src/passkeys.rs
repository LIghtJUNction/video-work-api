use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use thiserror::Error;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, CredentialID, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Webauthn, WebauthnBuilder, WebauthnResult,
};

const CEREMONY_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PENDING_CEREMONIES: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct PasskeyWebauthn {
    webauthn: Webauthn,
}

impl PasskeyWebauthn {
    pub(crate) fn new(rp_id: &str, origin: Url) -> WebauthnResult<Self> {
        let webauthn = WebauthnBuilder::new(rp_id, &origin)?
            .rp_name("Video Work API")
            .build()?;
        Ok(Self { webauthn })
    }

    pub(crate) fn start_passkey_registration(
        &self,
        user_unique_id: Uuid,
        user_name: &str,
        user_display_name: &str,
        exclude_credentials: Option<Vec<CredentialID>>,
    ) -> WebauthnResult<(CreationChallengeResponse, PasskeyRegistration)> {
        self.webauthn.start_passkey_registration(
            user_unique_id,
            user_name,
            user_display_name,
            exclude_credentials,
        )
    }

    pub(crate) fn finish_passkey_registration(
        &self,
        credential: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> WebauthnResult<Passkey> {
        self.webauthn.finish_passkey_registration(credential, state)
    }

    pub(crate) fn start_passkey_authentication(
        &self,
        passkeys: &[Passkey],
    ) -> WebauthnResult<(RequestChallengeResponse, PasskeyAuthentication)> {
        self.webauthn.start_passkey_authentication(passkeys)
    }

    pub(crate) fn finish_passkey_authentication(
        &self,
        credential: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> WebauthnResult<AuthenticationResult> {
        self.webauthn
            .finish_passkey_authentication(credential, state)
    }
}

#[derive(Debug)]
pub(crate) enum PendingCeremony {
    Registration {
        created_at: Instant,
        name: String,
        webauthn: PasskeyWebauthn,
        state: PasskeyRegistration,
    },
    Authentication {
        created_at: Instant,
        webauthn: PasskeyWebauthn,
        state: PasskeyAuthentication,
    },
}

impl PendingCeremony {
    pub(crate) fn registration(
        name: String,
        webauthn: PasskeyWebauthn,
        state: PasskeyRegistration,
    ) -> Self {
        Self::Registration {
            created_at: Instant::now(),
            name,
            webauthn,
            state,
        }
    }

    pub(crate) fn authentication(webauthn: PasskeyWebauthn, state: PasskeyAuthentication) -> Self {
        Self::Authentication {
            created_at: Instant::now(),
            webauthn,
            state,
        }
    }

    fn created_at(&self) -> Instant {
        match self {
            Self::Registration { created_at, .. } | Self::Authentication { created_at, .. } => {
                *created_at
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct CeremonyStore {
    pending: Mutex<HashMap<String, PendingCeremony>>,
}

#[derive(Debug, Error)]
pub(crate) enum CeremonyStoreError {
    #[error("too many pending passkey ceremonies")]
    Full,
    #[error("passkey ceremony store lock poisoned")]
    LockPoisoned,
}

impl CeremonyStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, PendingCeremony>>, CeremonyStoreError> {
        self.pending
            .lock()
            .map_err(|_| CeremonyStoreError::LockPoisoned)
    }

    pub(crate) fn insert(&self, ceremony: PendingCeremony) -> Result<String, CeremonyStoreError> {
        let mut pending = self.lock()?;
        Self::cleanup_locked(&mut pending, Instant::now());
        if pending.len() >= MAX_PENDING_CEREMONIES {
            return Err(CeremonyStoreError::Full);
        }
        let transaction_id = Uuid::new_v4().to_string();
        pending.insert(transaction_id.clone(), ceremony);
        Ok(transaction_id)
    }

    pub(crate) fn take(
        &self,
        transaction_id: &str,
    ) -> Result<Option<PendingCeremony>, CeremonyStoreError> {
        let now = Instant::now();
        let mut pending = self.lock()?;
        let ceremony = pending.remove(transaction_id);
        Self::cleanup_locked(&mut pending, now);
        Ok(ceremony.filter(|value| now.duration_since(value.created_at()) <= CEREMONY_TTL))
    }

    pub(crate) fn cleanup(&self) -> Result<(), CeremonyStoreError> {
        let mut pending = self.lock()?;
        Self::cleanup_locked(&mut pending, Instant::now());
        Ok(())
    }

    fn cleanup_locked(pending: &mut HashMap<String, PendingCeremony>, now: Instant) {
        pending.retain(|_, value| now.duration_since(value.created_at()) <= CEREMONY_TTL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registration(created_at: Instant) -> PendingCeremony {
        let origin = Url::parse("https://example.com").unwrap();
        let webauthn = PasskeyWebauthn::new("example.com", origin).unwrap();
        let (_, state) = webauthn
            .start_passkey_registration(Uuid::nil(), "admin", "Administrator", None)
            .unwrap();
        PendingCeremony::Registration {
            created_at,
            name: "test".into(),
            webauthn,
            state,
        }
    }

    #[test]
    fn domains_and_localhost_build_with_high_level_api() {
        for (rp_id, origin) in [
            ("example.com", "https://example.com"),
            ("localhost", "http://localhost:7860"),
        ] {
            assert!(PasskeyWebauthn::new(rp_id, Url::parse(origin).unwrap()).is_ok());
        }
    }

    #[test]
    fn ip_literal_origins_are_rejected_by_high_level_api() {
        for (rp_id, origin) in [
            ("127.0.0.1", "http://127.0.0.1:7860"),
            ("::1", "http://[::1]:7860"),
        ] {
            assert!(PasskeyWebauthn::new(rp_id, Url::parse(origin).unwrap()).is_err());
        }
    }

    #[test]
    fn take_consumes_ceremony_once() {
        let store = CeremonyStore::new();
        let id = store.insert(registration(Instant::now())).unwrap();
        assert!(store.take(&id).unwrap().is_some());
        assert!(store.take(&id).unwrap().is_none());
    }

    #[test]
    fn take_rejects_expired_ceremony() {
        let store = CeremonyStore::new();
        let id = store
            .insert(registration(
                Instant::now() - CEREMONY_TTL - Duration::from_secs(1),
            ))
            .unwrap();
        assert!(store.take(&id).unwrap().is_none());
    }

    #[test]
    fn insert_rejects_when_capacity_is_full() {
        let store = CeremonyStore::new();
        for _ in 0..MAX_PENDING_CEREMONIES {
            store.insert(registration(Instant::now())).unwrap();
        }
        assert!(matches!(
            store.insert(registration(Instant::now())),
            Err(CeremonyStoreError::Full)
        ));
    }

    #[test]
    fn expired_entry_frees_capacity() {
        let store = CeremonyStore::new();
        for _ in 0..MAX_PENDING_CEREMONIES - 1 {
            store.insert(registration(Instant::now())).unwrap();
        }
        store.pending.lock().unwrap().insert(
            "expired".into(),
            registration(Instant::now() - CEREMONY_TTL - Duration::from_secs(1)),
        );
        assert!(store.insert(registration(Instant::now())).is_ok());
    }
}
