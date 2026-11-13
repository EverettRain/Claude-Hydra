//! Per-account system identifiers for a sandboxed account.
//!
//! A Sandboxie box virtualizes files and the registry, but not what the system reports about
//! itself: the machine GUID, the hardware profile GUID, volume serials, MAC addresses, the
//! computer name and the account a boxed program reads are still this computer's, as are the
//! board, BIOS, memory and disk serials in the SMBIOS table and the storage descriptor (WMI, the
//! other route to them, is closed off in the box). So accounts kept side by side on one machine
//! look like one device. This module derives a stable set per account instead — the same account
//! always sees the same device, different accounts see different ones — and `sandbox::configure`
//! hands the values to the box, where `idguard.c` answers the matching system calls with them.
//!
//! The values are derived, not random: they stay the same as long as the account id does, so an
//! account does not look like new hardware after every Hydra update.

use sha2::{Digest, Sha256};

/// The identifiers a boxed account sees in place of this computer's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifiers {
    /// `HKLM\SOFTWARE\Microsoft\Cryptography`: `MachineGuid`, lowercase and unbraced, the way
    /// Windows stores it.
    pub machine_guid: String,
    /// `HKLM\SYSTEM\CurrentControlSet\Control\IDConfigDB\Hardware Profiles\0001`:
    /// `HwProfileGuid`, braced and uppercase.
    pub hw_profile_guid: String,
    /// Serial of the account's volumes (`GetVolumeInformationW`), 8 hex digits. This is the
    /// account's key; the serial a given volume reports is `volume_serial_for` with that
    /// volume's real serial, so two volumes of one account do not share one number.
    pub volume_serial: String,
    /// MAC address for the box's adapters (`GetAdaptersAddresses`), 12 hex digits. Locally
    /// administered (bit 1 of the first octet), so it cannot name real hardware.
    pub mac: String,
    /// Computer name (`GetComputerNameExW`, `GetComputerNameW`), shaped like a default Windows
    /// name and within the 15-character NetBIOS limit.
    pub computer_name: String,
    /// Account name (`GetUserNameW`, `GetUserNameExW`, `USERNAME`, `whoami`), shaped like a
    /// local account name. The guard replaces the name in the strings the system returns; it
    /// does not and cannot rename the profile folder behind it.
    pub user_name: String,
    /// The account's SID as text (`S-1-5-21-…-1001`). Derived from the account key, so two
    /// accounts on one machine do not report one identity. It is answered in both forms: the text
    /// entry points, the token the process reads (`GetTokenInformation`), and the LSA lookups .NET
    /// uses. The token still carries the host's SID for access checks - only what it reports is
    /// changed (see the status document).
    pub user_sid: String,
}

impl Identifiers {
    /// The identifiers of one account, which is also one box; stable for as long as the account
    /// id is.
    pub fn for_account(account_id: &str) -> Self {
        let key = crate::sandbox::account_key(account_id);
        let seed = key.as_bytes();
        Self {
            machine_guid: guid_text(&guid(seed, b"machine-guid"), false),
            hw_profile_guid: guid_text(&guid(seed, b"hw-profile-guid"), true),
            volume_serial: hex_text(&bytes(seed, b"volume-serial")[..4]).to_uppercase(),
            mac: mac_text(&bytes(seed, b"mac")),
            computer_name: computer_name(&bytes(seed, b"computer-name")),
            user_name: user_name(&bytes(seed, b"user-name")),
            user_sid: user_sid(&bytes(seed, b"user-sid")),
        }
    }
}

/// 32 derived bytes for one label of one account: labels and accounts never share a value, and
/// nothing about this computer enters the result.
fn bytes(seed: &[u8], label: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"hydra identity\0");
    hash.update(label);
    hash.update(b"\0");
    hash.update(seed);
    hash.finalize().into()
}

fn hex_text(digits: &[u8]) -> String { digits.iter().map(|b| format!("{b:02x}")).collect() }

/// The serial a boxed volume reports, as `GetVolumeInformationW` would return it, for the
/// volume whose real serial is `host_serial`.
///
/// The guard rewrites the serial where the system produces it (`NtQueryVolumeInformationFile`),
/// where the only thing known about the volume is its real serial — not a path. Keying on that
/// value makes every route to one volume agree: a drive letter, a `NULL` root (the current
/// drive), a folder mount point, an open file handle (`fs.stat().dev` in Node,
/// `GetFileInformationByHandle`). Each volume gets its own number, and a mount point under
/// `C:` is told apart from `C:` itself, as on real hardware.
///
/// The mix is keyed with the account's base value and nonlinear, so the relation between two
/// volumes' serials differs per account: XOR-ing an account's C: and D: serials does not give a
/// constant that would link accounts of one machine (or mark Hydra itself).
///
/// The rules must stay identical to `idguard.c::derive`; the Sandboxie test in `sandbox.rs`
/// compares what the box prints with what this function derives.
pub fn volume_serial_for(identifiers: &Identifiers, host_serial: u32) -> u32 {
    derive(&base_bytes(&identifiers.volume_serial), b'V', &host_serial.to_be_bytes())
}

/// The serial as the system prints it, `vol C:` style: two upper-case halves.
pub fn volume_serial_text(serial: u32) -> String {
    format!("{:04X}-{:04X}", serial >> 16, serial & 0xffff)
}

fn base_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("derived values are hex")).collect()
}

/// FNV-1a over `key || tag || data`, finished with the murmur3 avalanche. Mirrors `derive` in
/// `idguard.c` byte for byte.
pub fn derive(key: &[u8], tag: u8, data: &[u8]) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in key.iter().chain([tag].iter()).chain(data.iter()) {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(16_777_619);
    }
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}

/// Environment variables that carry the computer name or the account's own name, with the derived
/// `name` and `user` in place of the host's. On a machine outside a domain Windows sets
/// `USERDOMAIN` (and its roaming twin) to the computer name and `LOGONSERVER` to
/// `\\<computer name>`; replacing only `COMPUTERNAME` would leave the host's name one variable
/// away. On a domain-joined machine these name the domain and its controller, which every account
/// shares anyway, so they are kept. `USERNAME` is the account's name rather than the machine's, so
/// it follows the derived account for the same reason.
pub fn identity_env(
    name: &str,
    user: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let host = var("COMPUTERNAME").unwrap_or_default();
    let mut out = vec![
        ("COMPUTERNAME".to_string(), name.to_string()),
        ("USERNAME".to_string(), user.to_string()),
    ];
    for key in ["USERDOMAIN", "USERDOMAIN_ROAMINGPROFILE"] {
        if var(key).is_some_and(|v| !host.is_empty() && v.eq_ignore_ascii_case(&host)) {
            out.push((key.to_string(), name.to_string()));
        }
    }
    if var("LOGONSERVER").is_some_and(|v| !host.is_empty() && v.trim_start_matches('\\').eq_ignore_ascii_case(&host)) {
        out.push(("LOGONSERVER".to_string(), format!(r"\\{name}")));
    }
    out
}

/// A GUID from derived bytes, marked version 4 / RFC 4122 variant so its shape is the one the
/// system itself produces.
fn guid(seed: &[u8], label: &[u8]) -> [u8; 16] {
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes(seed, label)[..16]);
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

fn guid_text(id: &[u8; 16], braced: bool) -> String {
    let hex = hex_text(id);
    let text = format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
    // The hardware profile GUID is the braced, uppercase form the system writes; the machine
    // GUID is the bare, lowercase one.
    if braced { format!("{{{}}}", text.to_uppercase()) } else { text }
}

/// Six bytes shaped as a MAC: locally administered and unicast, so it never names real hardware.
fn mac_text(digits: &[u8]) -> String {
    let mut octets = [0u8; 6];
    octets.copy_from_slice(&digits[..6]);
    octets[0] = (octets[0] & 0xfc) | 0x02;
    hex_text(&octets)
}

/// `DESKTOP-` plus seven characters: the shape of a default Windows computer name.
fn computer_name(digits: &[u8]) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let suffix: String = digits[..7].iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    format!("DESKTOP-{suffix}")
}

/// A local account name: lower case, starts with a letter, no spaces or punctuation, six
/// characters. Windows compares account names case-insensitively, so a name that is already
/// lower case reads the same however a program prints it.
fn user_name(digits: &[u8]) -> String {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut name = String::new();
    name.push(LETTERS[digits[0] as usize % LETTERS.len()] as char);
    for byte in &digits[1..6] {
        name.push(REST[*byte as usize % REST.len()] as char);
    }
    name
}

/// A local account SID in the form Windows prints it: `S-1-5-21-<three authorities>-<rid>`.
///
/// The three sub-authorities are the machine part of the SID and the RID names the account, so
/// they must look like the ones a real installation derives — large, unremarkable numbers — or
/// the value marks itself out as made up. The RID is 1001, the first account a new installation
/// creates.
fn user_sid(digits: &[u8]) -> String {
    let authority = |at: usize| 200_000_000u32 + (u32::from_be_bytes([digits[at], digits[at + 1], digits[at + 2], digits[at + 3]]) % 1_500_000_000);
    format!(
        "S-1-5-21-{}-{}-{}-1001",
        authority(0),
        authority(4),
        authority(8)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_stable_per_account() {
        assert_eq!(Identifiers::for_account("account-2"), Identifiers::for_account("account-2"));
        let one = Identifiers::for_account("account-1");
        let two = Identifiers::for_account("account-2");
        assert_ne!(one.machine_guid, two.machine_guid);
        assert_ne!(one.hw_profile_guid, two.hw_profile_guid);
        assert_ne!(one.volume_serial, two.volume_serial);
        assert_ne!(one.mac, two.mac);
    }

    #[test]
    fn identifiers_have_the_shape_the_system_reports() {
        let ids = Identifiers::for_account("env-1a1cef07923");
        assert_eq!(ids.machine_guid.len(), 36);
        assert!(ids.machine_guid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(ids.machine_guid.chars().all(|c| !c.is_ascii_uppercase()));
        assert_eq!(ids.hw_profile_guid, format!("{{{}}}", ids.hw_profile_guid.trim_matches(['{', '}'])).to_uppercase());
        assert_eq!(ids.volume_serial.len(), 8);
        assert!(ids.volume_serial.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(ids.mac.len(), 12);
        let first = u8::from_str_radix(&ids.mac[..2], 16).expect("MAC starts with hex");
        assert_eq!(first & 0x03, 0x02, "a synthetic MAC is locally administered and unicast");
        assert!(ids.computer_name.starts_with("DESKTOP-"));
        assert_eq!(ids.computer_name.len(), 15, "a NetBIOS name holds at most 15 characters");
        assert!(ids.computer_name.chars().all(|c| !c.is_ascii_lowercase()));
    }

    #[test]
    fn guids_are_version_four() {
        let ids = Identifiers::for_account("account-2");
        assert_eq!(&ids.machine_guid[14..15], "4");
        let variant = u8::from_str_radix(&ids.machine_guid[19..20], 16).expect("variant digit is hex");
        assert!(variant >= 8 && variant <= 11, "RFC 4122 variant");
    }

    #[test]
    fn volumes_of_one_account_get_their_own_serial() {
        let ids = Identifiers::for_account("account-2");
        let c = volume_serial_for(&ids, 0x04EF_9568);
        let d = volume_serial_for(&ids, 0x3E22_487B);
        assert_ne!(c, d, "two volumes must not report one serial: {}", volume_serial_text(c));
        assert_ne!(c, 0x04EF_9568, "the host serial must not come through");
        assert_eq!(c, volume_serial_for(&ids, 0x04EF_9568), "stable per volume");
        assert_eq!(volume_serial_text(c).len(), 9);
        assert_eq!(&volume_serial_text(c)[4..5], "-");
    }

    #[test]
    fn volume_serials_differ_between_accounts() {
        let one = Identifiers::for_account("account-1");
        let two = Identifiers::for_account("account-2");
        assert_ne!(volume_serial_for(&one, 0x04EF_9568), volume_serial_for(&two, 0x04EF_9568));
    }

    #[test]
    fn volume_relations_do_not_link_accounts() {
        // With a plain XOR scheme `serial(C) ^ serial(D)` is the same for every account on one
        // machine, which would tie them together. The keyed mix must not have that property.
        let (c, d) = (0x04EF_9568, 0x3E22_487B);
        let one = Identifiers::for_account("account-1");
        let two = Identifiers::for_account("account-2");
        assert_ne!(
            volume_serial_for(&one, c) ^ volume_serial_for(&one, d),
            volume_serial_for(&two, c) ^ volume_serial_for(&two, d)
        );
    }

    #[test]
    fn derive_matches_the_guard() {
        // Observed in a real box on 2026-11-12: the C guard turned the host volume serial
        // 04EF-9568 into 89A3-60FA for this account (`vol C:` from 32-bit cmd, and
        // `fs.statSync('C:/').dev` = 2309185786 from 64-bit Node). A change to either side of
        // the derivation must fail here, not only in the Sandboxie test.
        let ids = Identifiers::for_account("hydra-identity-test");
        assert_eq!(volume_serial_for(&ids, 0x04EF_9568), 0x89A3_60FA);
        assert_ne!(derive(&[1, 2, 3, 4], b'V', &[0, 0, 0, 1]), derive(&[1, 2, 3, 4], b'H', &[0, 0, 0, 1]));
    }

    #[test]
    fn account_variables_follow_the_derived_identity() {
        let host = |k: &str| match k {
            "COMPUTERNAME" => Some("LAPTOP-UUCIGDOF".to_string()),
            "USERDOMAIN" | "USERDOMAIN_ROAMINGPROFILE" => Some("LAPTOP-UUCIGDOF".to_string()),
            "LOGONSERVER" => Some(r"\\LAPTOP-UUCIGDOF".to_string()),
            "USERNAME" => Some("24779".to_string()),
            _ => None,
        };
        let env = identity_env("DESKTOP-ABC1234", "abcdef", host);
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("COMPUTERNAME"), Some("DESKTOP-ABC1234"));
        assert_eq!(get("USERNAME"), Some("abcdef"));
        assert_eq!(get("USERDOMAIN"), Some("DESKTOP-ABC1234"));
        assert_eq!(get("USERDOMAIN_ROAMINGPROFILE"), Some("DESKTOP-ABC1234"));
        assert_eq!(get("LOGONSERVER"), Some(r"\\DESKTOP-ABC1234"));
        // A domain is shared by every account and is not the machine's name: left alone.
        let domain = |k: &str| match k {
            "COMPUTERNAME" => Some("LAPTOP-UUCIGDOF".to_string()),
            "USERDOMAIN" => Some("CORP".to_string()),
            "LOGONSERVER" => Some(r"\\DC01".to_string()),
            _ => None,
        };
        assert_eq!(identity_env("DESKTOP-ABC1234", "abcdef", domain).len(), 2);
    }
}
