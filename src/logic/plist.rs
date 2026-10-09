//! Apple property lists (XML and binary) ⇄ JSON, and the provisioning
//! profiles that wrap one.

use std::io::Cursor;

use base64::Engine as _;
use plist::Value as Plist;
use serde_json::{Map, Value};

fn friendly(e: plist::Error) -> String {
    let msg = e.to_string();
    let mut c = msg.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(msg)
}

/// A property list in either encoding.
pub fn parse(bytes: &[u8]) -> Result<Plist, String> {
    Plist::from_reader(Cursor::new(bytes)).map_err(|e| format!("Not a property list: {}", friendly(e)))
}

fn date_text(d: plist::Date) -> String {
    let t: chrono::DateTime<chrono::Utc> = std::time::SystemTime::from(d).into();
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// JSON has no dates or raw bytes: dates become ISO 8601 text and data Base64.
pub fn to_json(v: &Plist) -> Value {
    match v {
        Plist::Array(a) => Value::Array(a.iter().map(to_json).collect()),
        Plist::Dictionary(d) => Value::Object(d.iter().map(|(k, x)| (k.clone(), to_json(x))).collect()),
        Plist::Boolean(b) => (*b).into(),
        Plist::Data(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes).into(),
        Plist::Date(d) => date_text(*d).into(),
        Plist::Real(f) => serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or_else(|| f.to_string().into()),
        Plist::Integer(i) => match (i.as_signed(), i.as_unsigned()) {
            (Some(n), _) => n.into(),
            (None, Some(n)) => n.into(),
            _ => Value::Null,
        },
        Plist::String(s) => s.clone().into(),
        Plist::Uid(u) => {
            let mut m = Map::new();
            m.insert("CF$UID".into(), u.get().into());
            m.into()
        }
        _ => Value::Null,
    }
}

fn from_json(v: &Value, path: &str) -> Result<Plist, String> {
    Ok(match v {
        Value::Null => return Err(format!("Property lists have no null, found one at {}", if path.is_empty() { "the top level" } else { path })),
        Value::Bool(b) => Plist::Boolean(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => Plist::Integer(i.into()),
            (None, Some(u)) => Plist::Integer(u.into()),
            _ => Plist::Real(n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(s) => Plist::String(s.clone()),
        Value::Array(a) => Plist::Array(a.iter().enumerate().map(|(i, x)| from_json(x, &format!("{path}[{i}]"))).collect::<Result<_, _>>()?),
        Value::Object(m) => {
            let mut d = plist::Dictionary::new();
            for (k, x) in m {
                let at = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                d.insert(k.clone(), from_json(x, &at)?);
            }
            Plist::Dictionary(d)
        }
    })
}

/// The list as XML text: tab-indented the way Xcode writes it, or with two
/// spaces per level for an editor that cannot show tabs.
pub fn to_xml(v: &Plist, tabs: bool) -> Result<String, String> {
    let mut buf = Vec::new();
    let options = if tabs { plist::XmlWriteOptions::default() } else { plist::XmlWriteOptions::default().indent(b' ', 2) };
    v.to_writer_xml_with_options(&mut buf, &options).map_err(friendly)?;
    String::from_utf8(buf).map(|s| s.trim_end().to_string()).map_err(|e| e.to_string())
}

pub fn plist_to_json(src: &str) -> Result<Value, String> {
    Ok(to_json(&parse(src.trim_start().as_bytes())?))
}

pub fn json_to_plist(v: &Value) -> Result<String, String> {
    to_xml(&from_json(v, "")?, true)
}

// ---------------------------------------------------------------- provisioning profiles

#[derive(Debug)]
pub struct ProfileCert {
    pub name: String,
    pub expires: String,
    pub days_left: i64,
    pub sha1: String,
    /// It runs out while the profile is still valid.
    pub before_profile: bool,
}

#[derive(Debug)]
pub struct Profile {
    pub name: String,
    /// Development, Ad Hoc, App Store, Enterprise or Developer ID.
    pub kind: &'static str,
    /// Whether a build signed with it can be submitted to the App Store.
    pub store_ready: bool,
    /// Whole days until the profile expires; negative once expired.
    pub days_left: i64,
    pub rows: Vec<(&'static str, String)>,
    pub entitlements: Vec<(String, String)>,
    pub devices: Vec<String>,
    pub certificates: Vec<ProfileCert>,
    pub warnings: Vec<String>,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Entitlement values on one line: lists joined, nested values as JSON.
fn brief(v: &Plist) -> String {
    match v {
        Plist::Boolean(b) => b.to_string(),
        Plist::String(s) => s.clone(),
        Plist::Array(a) => a.iter().map(brief).collect::<Vec<_>>().join(", "),
        other => to_json(other).to_string(),
    }
}

fn strings(v: Option<&Plist>) -> Vec<String> {
    v.and_then(Plist::as_array).map(|a| a.iter().filter_map(|x| x.as_string().map(str::to_string)).collect()).unwrap_or_default()
}

/// Read a `.mobileprovision` / `.provisionprofile`: a signed (CMS) envelope
/// around a property list, which is stored inside as plain XML.
pub fn inspect_profile(bytes: &[u8]) -> Result<Profile, String> {
    let list = match (find(bytes, b"<?xml"), find(bytes, b"</plist>")) {
        (Some(a), Some(b)) if a < b => parse(&bytes[a..b + 8])?,
        _ => parse(bytes).map_err(|_| "This is not a provisioning profile: there is no property list inside it".to_string())?,
    };
    let dict = list.as_dictionary().ok_or("The profile's property list is not a dictionary")?;
    let text = |k: &str| dict.get(k).and_then(Plist::as_string).map(str::to_string);
    let expiry = dict.get("ExpirationDate").and_then(Plist::as_date).ok_or("This property list is not a provisioning profile: it has no ExpirationDate")?;
    let ents = dict.get("Entitlements").and_then(Plist::as_dictionary);
    let flag = |k: &str| ents.and_then(|e| e.get(k)).and_then(Plist::as_boolean).unwrap_or(false);

    let expires: chrono::DateTime<chrono::Utc> = std::time::SystemTime::from(expiry).into();
    let now = chrono::Utc::now();
    let days_left = (expires.timestamp() - now.timestamp()).div_euclid(86_400);

    let mut certificates = Vec::new();
    for der in dict.get("DeveloperCertificates").and_then(Plist::as_array).into_iter().flatten().filter_map(Plist::as_data) {
        let Ok(c) = super::cert::describe(der) else { continue };
        let row = |k: &str| c.rows.iter().find(|r| r.0 == k).map(|r| r.1.clone()).unwrap_or_default();
        certificates.push(ProfileCert {
            name: c.title.clone(),
            expires: row("Valid until"),
            days_left: c.days_left,
            sha1: row("SHA-1 fingerprint"),
            before_profile: c.expires < expires.timestamp(),
        });
    }

    let devices = strings(dict.get("ProvisionedDevices"));
    let all_devices = dict.get("ProvisionsAllDevices").and_then(Plist::as_boolean).unwrap_or(false);
    let developer_id = certificates.iter().any(|c| c.name.starts_with("Developer ID"));
    let kind = if all_devices {
        "Enterprise (In-House)"
    } else if flag("get-task-allow") {
        "Development"
    } else if !devices.is_empty() {
        "Ad Hoc"
    } else if developer_id {
        "Developer ID"
    } else {
        "App Store"
    };
    let store_ready = kind == "App Store";

    let mut rows: Vec<(&'static str, String)> = Vec::new();
    let mut push = |k: &'static str, v: Option<String>| rows.extend(v.filter(|v| !v.is_empty()).map(|v| (k, v)));
    push("Type", Some(kind.to_string()));
    push("App ID name", text("AppIDName"));
    let app_id = ents.and_then(|e| e.get("application-identifier").or_else(|| e.get("com.apple.application-identifier"))).and_then(Plist::as_string);
    push("App ID", app_id.map(str::to_string));
    push("Team", text("TeamName"));
    push("Team ID", Some(strings(dict.get("TeamIdentifier")).join(", ")));
    push("Platform", Some(strings(dict.get("Platform")).join(", ")));
    push("UUID", text("UUID"));
    push("Created", dict.get("CreationDate").and_then(Plist::as_date).map(date_text));
    push("Expires", Some(date_text(expiry)));
    push("Devices", Some(if all_devices { "All devices".into() } else if devices.is_empty() { String::new() } else { super::plural(devices.len(), "device") }));
    push("Managed by Xcode", dict.get("IsXcodeManaged").and_then(Plist::as_boolean).map(|b| if b { "Yes".into() } else { "No".into() }));

    let entitlements = ents.map(|e| e.iter().map(|(k, v)| (k.clone(), brief(v))).collect()).unwrap_or_default();

    let mut warnings = Vec::new();
    if days_left < 0 {
        warnings.push(format!("The profile expired {} ago", super::plural(-days_left as usize, "day")));
    } else if days_left < 30 {
        warnings.push(format!("The profile expires in {}", super::plural(days_left as usize, "day")));
    }
    for c in certificates.iter().filter(|c| c.before_profile && days_left >= 0) {
        let when = if c.days_left < 0 { "has already expired".to_string() } else { format!("expires in {}", super::plural(c.days_left as usize, "day")) };
        warnings.push(format!("The certificate \"{}\" {when}, before the profile does", c.name));
    }
    if certificates.is_empty() {
        warnings.push("The profile lists no signing certificates".into());
    }
    if !store_ready {
        warnings.push(format!("{kind} profiles cannot be used to submit to the App Store"));
    }

    Ok(Profile { name: text("Name").unwrap_or_else(|| "Untitled profile".into()), kind, store_ready, days_left, rows, entitlements, devices, certificates, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
	<key>CFBundleName</key>
	<string>SideKit</string>
	<key>LSMinimumSystemVersion</key>
	<real>12.5</real>
	<key>Build</key>
	<integer>42</integer>
	<key>Sandbox</key>
	<true/>
	<key>Built</key>
	<date>2026-10-01T12:00:00Z</date>
	<key>Icon</key>
	<data>AQID</data>
	<key>Schemes</key>
	<array>
		<string>sidekit</string>
		<dict>
			<key>role</key>
			<string>Editor</string>
		</dict>
	</array>
</dict>
</plist>";

    #[test]
    fn xml_to_json_and_back() {
        let v = plist_to_json(XML).unwrap();
        assert_eq!(
            v,
            json!({"CFBundleName": "SideKit", "LSMinimumSystemVersion": 12.5, "Build": 42, "Sandbox": true, "Built": "2026-10-01T12:00:00Z", "Icon": "AQID", "Schemes": ["sidekit", {"role": "Editor"}]})
        );
        // Key order survives.
        assert!(v.to_string().starts_with("{\"CFBundleName\":\"SideKit\",\"LSMinimumSystemVersion\""));
        let xml = json_to_plist(&json!({"name": "a & b", "n": 1, "f": 1.5, "on": false, "list": [1, {"k": "v"}]})).unwrap();
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist"), "{xml}");
        assert!(xml.contains("\t<key>name</key>\n\t<string>a &amp; b</string>\n\t<key>n</key>\n\t<integer>1</integer>\n\t<key>f</key>\n\t<real>1.5</real>\n\t<key>on</key>\n\t<false/>"), "{xml}");
        assert!(xml.ends_with("</plist>"));
        assert_eq!(plist_to_json(&xml).unwrap(), json!({"name": "a & b", "n": 1, "f": 1.5, "on": false, "list": [1, {"k": "v"}]}));
        assert_eq!(json_to_plist(&json!({"a": [1, null]})).unwrap_err(), "Property lists have no null, found one at a[1]");
        assert!(plist_to_json("hello").unwrap_err().starts_with("Not a property list"));
    }

    /// The smallest binary plist holding `{"a": true, "n": 7}`.
    fn binary() -> Vec<u8> {
        let mut b = b"bplist00".to_vec();
        // Objects: dict(2), "a", "n", true, int 7.
        b.extend([0xD2, 1, 2, 3, 4]); // offset 8
        b.extend([0x51, b'a']); // 13
        b.extend([0x51, b'n']); // 15
        b.push(0x09); // 17
        b.extend([0x10, 7]); // 18
        b.extend([8, 13, 15, 17, 18]); // offset table at 20
        b.extend([0u8; 6]);
        b.extend([1, 1]); // offset size, reference size
        b.extend(5u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(20u64.to_be_bytes());
        b
    }

    #[test]
    fn reads_binary_lists() {
        let v = parse(&binary()).unwrap();
        assert_eq!(to_json(&v), json!({"a": true, "n": 7}));
        assert!(to_xml(&v, true).unwrap().contains("<key>n</key>\n\t<integer>7</integer>"));
        assert!(to_xml(&v, false).unwrap().contains("<key>n</key>\n  <integer>7</integer>"));
    }

    fn profile(extra: &str, expires: &str) -> Vec<u8> {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>Name</key><string>SideKit Dev</string><key>AppIDName</key><string>SideKit</string><key>TeamName</key><string>Example Ltd</string><key>TeamIdentifier</key><array><string>ABCDE12345</string></array><key>UUID</key><string>11111111-2222-3333-4444-555555555555</string><key>Platform</key><array><string>iOS</string></array><key>CreationDate</key><date>2026-01-01T00:00:00Z</date><key>ExpirationDate</key><date>{expires}</date>{extra}</dict></plist>"
        );
        // A real profile is this XML inside a signed binary envelope.
        let mut bytes = vec![0x30, 0x82, 0x1f, 0x00, 0xff];
        bytes.extend(xml.as_bytes());
        bytes.extend([0xa0, 0x82, 0x0d]);
        bytes
    }

    #[test]
    fn inspects_profiles() {
        let dev = "<key>ProvisionedDevices</key><array><string>00008030-A</string><string>00008030-B</string></array><key>Entitlements</key><dict><key>application-identifier</key><string>ABCDE12345.com.example.sidekit</string><key>get-task-allow</key><true/><key>keychain-access-groups</key><array><string>ABCDE12345.*</string><string>com.apple.token</string></array></dict>";
        let p = inspect_profile(&profile(dev, "2099-01-01T00:00:00Z")).unwrap();
        assert_eq!((p.name.as_str(), p.kind, p.store_ready), ("SideKit Dev", "Development", false));
        let get = |k: &str| p.rows.iter().find(|r| r.0 == k).map(|r| r.1.as_str());
        assert_eq!(get("App ID"), Some("ABCDE12345.com.example.sidekit"));
        assert_eq!(get("Team ID"), Some("ABCDE12345"));
        assert_eq!(get("Devices"), Some("2 devices"));
        assert_eq!(get("Expires"), Some("2099-01-01T00:00:00Z"));
        assert_eq!(p.devices, ["00008030-A", "00008030-B"]);
        assert_eq!(p.entitlements[1], ("get-task-allow".to_string(), "true".to_string()));
        assert_eq!(p.entitlements[2].1, "ABCDE12345.*, com.apple.token");
        assert!(p.days_left > 365);
        assert_eq!(p.warnings, ["The profile lists no signing certificates", "Development profiles cannot be used to submit to the App Store"]);

        let kind = |extra: &str| inspect_profile(&profile(extra, "2099-01-01T00:00:00Z")).unwrap().kind;
        assert_eq!(kind("<key>ProvisionedDevices</key><array><string>X</string></array>"), "Ad Hoc");
        assert_eq!(kind("<key>ProvisionsAllDevices</key><true/>"), "Enterprise (In-House)");
        assert_eq!(kind(""), "App Store");

        let old = inspect_profile(&profile("", "2020-01-01T00:00:00Z")).unwrap();
        assert!(old.store_ready && old.days_left < 0);
        assert!(old.warnings[0].starts_with("The profile expired "));
        assert!(inspect_profile(b"not a profile").is_err());
        assert!(inspect_profile(XML.as_bytes()).unwrap_err().contains("no ExpirationDate"));
    }

    #[test]
    fn flags_certificates_that_expire_first() {
        // The test certificate from the certificate decoder runs until 2036.
        let der = base64::engine::general_purpose::STANDARD
            .decode("MIIB2TCCAYCgAwIBAgIUT+smGBqk9DX+i3hgl5lA/zaqMiMwCgYIKoZIzj0EAwIwKTEVMBMGA1UEAwwMc2lkZWtpdC50ZXN0MRAwDgYDVQQKDAdTaWRlS2l0MB4XDTI2MDkyNDE4MzkzNVoXDTM2MDkyMTE4MzkzNVowKTEVMBMGA1UEAwwMc2lkZWtpdC50ZXN0MRAwDgYDVQQKDAdTaWRlS2l0MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEo8Fg+CSac5RXxTMoiTldThP42mIW2mJi7JL1dComWlko+ECLlXZEFtGXizr8POgjGWi3ReBkZR8bY8bQuioW4aOBhTCBgjAdBgNVHQ4EFgQUq1ptnmODM4kkTX8EXRQdku/rNBEwHwYDVR0jBBgwFoAUq1ptnmODM4kkTX8EXRQdku/rNBEwDwYDVR0TAQH/BAUwAwEB/zAvBgNVHREEKDAmggxzaWRla2l0LnRlc3SCEHd3dy5zaWRla2l0LnRlc3SHBH8AAAEwCgYIKoZIzj0EAwIDRwAwRAIgUq7tFFWcJV+3FIjrjw8kQqsqsi16yTBGQ3WdJQ52h78CIGMOo6rASsM/WkZU0qLIKkt7IehRRgRaWVlq2Q30vqFO")
            .unwrap();
        let certs = format!("<key>DeveloperCertificates</key><array><data>{}</data></array>", base64::engine::general_purpose::STANDARD.encode(&der));
        let p = inspect_profile(&profile(&certs, "2099-01-01T00:00:00Z")).unwrap();
        assert_eq!(p.certificates.len(), 1);
        assert_eq!((p.certificates[0].name.as_str(), p.certificates[0].before_profile), ("sidekit.test", true));
        assert!(p.warnings[0].starts_with("The certificate \"sidekit.test\" expires in ") && p.warnings[0].ends_with(", before the profile does"));
        let fine = inspect_profile(&profile(&certs, "2030-01-01T00:00:00Z")).unwrap();
        assert!(!fine.certificates[0].before_profile && fine.warnings.is_empty());
    }
}
