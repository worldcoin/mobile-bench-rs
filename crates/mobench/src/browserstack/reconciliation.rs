//! Requested-to-observed BrowserStack device reconciliation.
//!
//! Requested selectors remain stable run-plan identity. Observed identities
//! come from authenticated provider responses and are matched without ever
//! rewriting either side.

use anyhow::{Result, anyhow};

use super::DeviceSession;
#[cfg(test)]
use super::{BrowserStackDevice, SessionDetails};

#[derive(Clone, Debug)]
pub(super) struct ReconciledDeviceSession {
    pub(super) requested_device_id: String,
    pub(super) observed: DeviceSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceSelector<'a> {
    name: &'a str,
    version: Vec<u32>,
}

fn parse_device_selector(selector: &str) -> Option<DeviceSelector<'_>> {
    let (name, version) = selector.rsplit_once('-')?;
    let version = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    (!name.is_empty() && !version.is_empty()).then_some(DeviceSelector { name, version })
}

pub(super) fn is_valid_device_selector(selector: &str) -> bool {
    parse_device_selector(selector).is_some()
}

pub(super) fn requested_selector_matches_observed(requested: &str, observed: &str) -> bool {
    let (Some(requested), Some(observed)) = (
        parse_device_selector(requested),
        parse_device_selector(observed),
    ) else {
        return false;
    };
    if requested.name != observed.name {
        return false;
    }
    if requested.version.len() == 1 {
        requested.version.first() == observed.version.first()
    } else {
        requested.version == observed.version
    }
}

pub(super) fn reconcile_requested_device_sessions(
    requested_devices: &[String],
    observed_sessions: &[DeviceSession],
) -> Result<Vec<ReconciledDeviceSession>> {
    let mut unique_requested = std::collections::BTreeSet::new();
    for requested in requested_devices {
        if parse_device_selector(requested).is_none() {
            return Err(anyhow!(
                "BrowserStack requested device selector is invalid: {requested}"
            ));
        }
        if !unique_requested.insert(requested.as_str()) {
            return Err(anyhow!(
                "BrowserStack requested device selector is duplicated: {requested}"
            ));
        }
    }

    let mut unmatched = (0..observed_sessions.len()).collect::<Vec<_>>();
    let mut reconciled = Vec::with_capacity(requested_devices.len());
    for requested in requested_devices {
        let candidates = unmatched
            .iter()
            .copied()
            .filter(|index| {
                requested_selector_matches_observed(requested, &observed_sessions[*index].device)
            })
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => {
                return Err(anyhow!(
                    "BrowserStack requested device had no compatible observed session: {requested}; observed [{}]",
                    observed_sessions
                        .iter()
                        .map(|session| session.device.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            [index] => {
                reconciled.push(ReconciledDeviceSession {
                    requested_device_id: requested.clone(),
                    observed: observed_sessions[*index].clone(),
                });
                unmatched.retain(|candidate| candidate != index);
            }
            _ => {
                return Err(anyhow!(
                    "BrowserStack requested device matched multiple observed sessions: {requested}; candidates [{}]",
                    candidates
                        .iter()
                        .map(|index| observed_sessions[*index].device.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    if !unmatched.is_empty() {
        return Err(anyhow!(
            "BrowserStack returned unexpected observed sessions: [{}]",
            unmatched
                .iter()
                .map(|index| observed_sessions[*index].device.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(reconciled)
}

#[cfg(test)]
pub(super) fn provider_device_identifier(details: &SessionDetails) -> String {
    BrowserStackDevice {
        device: details.device.clone(),
        os: details.os.clone(),
        os_version: details.os_version.clone(),
        available: None,
    }
    .identifier()
}
