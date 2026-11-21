use anyhow::{Result, bail};
use axum::http::{HeaderMap, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::{SignedDuration, Timestamp};
use qrcode::QrCode;
use qrcode::render::unicode;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::clock::iso;
use crate::config::RemoteOrigin;
use crate::store::{BoardSession, RedeemResult, Store};

pub const SESSION_COOKIE: &str = "__Host-t3cm_session";
pub const PAIRING_TTL_SECS: i64 = 5 * 60;
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;
pub const MAX_LABEL_LEN: usize = 80;
const PRUNE_AFTER_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MintedPairing {
    pub url: String,
    #[serde(skip)]
    pub token: String,
    pub label: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub ttl_seconds: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieExtract<'a> {
    Missing,
    Present(&'a str),
    Invalid,
}

pub fn hash_token(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("system randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn format_session_id(seq: i64) -> String {
    format!("s{seq}")
}

pub fn parse_session_id(raw: &str) -> Option<i64> {
    let digits = raw.strip_prefix('s')?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().filter(|&n| n > 0)
}

pub fn clean_label(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().any(|c| c.is_control()) {
        bail!("session label must be a single line with no control characters");
    }
    if trimmed.chars().count() > MAX_LABEL_LEN {
        bail!("session label must be at most {MAX_LABEL_LEN} characters");
    }
    Ok(Some(trimmed.to_string()))
}

pub fn mint_pairing_token(
    store: &Store,
    origin: &RemoteOrigin,
    label: Option<&str>,
    now: Timestamp,
) -> Result<MintedPairing> {
    let label = clean_label(label)?;
    let token = generate_token();
    let token_hash = hash_token(&token);
    let created_at = iso(now);
    let expires_at = iso(now.checked_add(SignedDuration::from_secs(PAIRING_TTL_SECS)).unwrap_or(now));
    let prune_before = iso(now.checked_sub(SignedDuration::from_secs(PRUNE_AFTER_SECS)).unwrap_or(now));
    store.pairing_token_insert(&token_hash, label.as_deref(), &created_at, &expires_at, &prune_before)?;
    let url = format!("{}/pair#{token}", origin.canonical());
    Ok(MintedPairing { url, token, label, created_at, expires_at, ttl_seconds: PAIRING_TTL_SECS })
}

pub fn render_qr(url: &str) -> Result<String> {
    let code = QrCode::new(url.as_bytes())?;
    Ok(code.render::<unicode::Dense1x2>().quiet_zone(true).build())
}

pub fn redeem_pairing_token(
    store: &Store,
    raw_token: &str,
    request_label: Option<&str>,
    now: Timestamp,
) -> Result<Option<(String, BoardSession)>> {
    let raw_token = raw_token.trim();
    if raw_token.is_empty() {
        return Ok(None);
    }
    let token_hash = hash_token(raw_token);
    let raw_session = generate_token();
    let session_hash = hash_token(&raw_session);
    let now_iso = iso(now);
    let session_expires = iso(now.checked_add(SignedDuration::from_secs(SESSION_TTL_SECS)).unwrap_or(now));
    match store.pairing_token_redeem(&token_hash, &session_hash, request_label, &now_iso, &session_expires)? {
        RedeemResult::Redeemed(session) => Ok(Some((raw_session, session))),
        RedeemResult::NotFound | RedeemResult::AlreadyUsed | RedeemResult::Expired => Ok(None),
    }
}

pub fn session_cookie_header(raw_session: &str) -> String {
    format!("{SESSION_COOKIE}={raw_session}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}")
}

pub fn extract_session_cookie(headers: &HeaderMap) -> CookieExtract<'_> {
    let mut found: Option<&str> = None;
    for raw in headers.get_all(header::COOKIE) {
        let Ok(s) = raw.to_str() else {
            return CookieExtract::Invalid;
        };
        for part in s.split(';') {
            let part = part.trim();
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            if name.trim() == SESSION_COOKIE {
                let value = value.trim();
                if found.is_some() || value.is_empty() {
                    return CookieExtract::Invalid;
                }
                found = Some(value);
            }
        }
    }
    match found {
        Some(v) => CookieExtract::Present(v),
        None => CookieExtract::Missing,
    }
}
