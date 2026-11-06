use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

const CAREERS: &str = "https://discord.com/careers";
const PREFIX: usize = 0x10000;

fn sha(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn xor(mut out: Vec<u8>, user_id: &str) -> String {
    let key = sha(format!("{user_id}{CAREERS}").as_bytes());
    for i in 1..out.len() {
        out[i] ^= key[(i - 1) & 31];
    }
    STANDARD.encode(out)
}

fn payload(
    flags: u8,
    file_size: u64,
    timestamp: u32,
    size_of_image: u32,
    header: [u8; 32],
    machine_user: [u8; 32],
    section: [u8; 32],
    cert: [u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(146);
    out.push(1);
    out.push(flags);
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&timestamp.to_le_bytes());
    out.extend_from_slice(&size_of_image.to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&machine_user);
    out.extend_from_slice(&section);
    out.extend_from_slice(&cert);
    out
}

fn machine_user(machine_guid: &str, user_id: &str) -> [u8; 32] {
    sha(format!("{machine_guid}{user_id}").as_bytes())
}

pub fn machine_guid() -> String {
    if let Ok(value) = std::env::var("BADGE_MACHINE_GUID") {
        if !value.trim().is_empty() {
            return value.trim().to_string();
        }
    }

    let out = Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output();

    if let Ok(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = text.lines().find(|l| l.contains("MachineGuid")) {
            if let Some(value) = line.split_whitespace().last() {
                if value.len() >= 32 {
                    return value.to_string();
                }
            }
        }
    }

    if let Ok(text) = std::fs::read_to_string("/etc/machine-id") {
        let value = text.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    let path = std::env::var("HOME")
        .map(|home| format!("{home}/.badge-farm-machine-guid"))
        .unwrap_or_else(|_| ".badge-farm-machine-guid".to_string());

    if let Ok(text) = std::fs::read_to_string(&path) {
        let value = text.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    let value = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::write(&path, &value);
    value
}

#[cfg(windows)]
fn signature(blob: &[u8]) -> Option<[u8; 32]> {
    use windows_sys::Win32::Security::Cryptography::*;

    #[repr(C)]
    struct Blob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    let mut data = Blob {
        cb_data: blob.len() as u32,
        pb_data: blob.as_ptr() as *mut u8,
    };

    unsafe {
        let mut encoding = 0u32;
        let mut content = 0u32;
        let mut format = 0u32;
        let mut store: HCERTSTORE = std::ptr::null_mut();
        let mut msg: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut context: *mut core::ffi::c_void = std::ptr::null_mut();

        let ok = CryptQueryObject(
            CERT_QUERY_OBJECT_BLOB,
            std::ptr::addr_of_mut!(data) as *const core::ffi::c_void,
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            &mut encoding,
            &mut content,
            &mut format,
            &mut store,
            &mut msg,
            &mut context,
        );
        if ok == 0 {
            return None;
        }

        let mut size = 0u32;
        if CryptMsgGetParam(
            msg,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            std::ptr::null_mut(),
            &mut size,
        ) == 0
            || size == 0
        {
            CryptMsgClose(msg);
            CertCloseStore(store, 0);
            return None;
        }

        let mut info = vec![0u8; size as usize];
        if CryptMsgGetParam(
            msg,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            info.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size,
        ) == 0
        {
            CryptMsgClose(msg);
            CertCloseStore(store, 0);
            return None;
        }

        let cert = CertFindCertificateInStore(
            store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            info.as_ptr() as *const core::ffi::c_void,
            std::ptr::null(),
        );
        if cert.is_null() {
            CryptMsgClose(msg);
            CertCloseStore(store, 0);
            return None;
        }

        let mut name = vec![0u16; 256];
        let len = CertGetNameStringW(
            cert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            std::ptr::null(),
            name.as_mut_ptr(),
            name.len() as u32,
        );
        if len == 0 {
            CertFreeCertificateContext(cert);
            CryptMsgClose(msg);
            CertCloseStore(store, 0);
            return None;
        }

        let mut hash = [0u8; 32];
        let mut hash_len = 32u32;
        if CryptHashPublicKeyInfo(
            0usize,
            CALG_SHA_256,
            0,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            std::ptr::addr_of!((*(*cert).pCertInfo).SubjectPublicKeyInfo),
            hash.as_mut_ptr(),
            &mut hash_len,
        ) == 0
            || hash_len != 32
        {
            CertFreeCertificateContext(cert);
            CryptMsgClose(msg);
            CertCloseStore(store, 0);
            return None;
        }

        let subject = String::from_utf16_lossy(&name[..len as usize - 1]);
        let mut hasher = Sha256::new();
        hasher.update(subject.as_bytes());
        hasher.update(hash);
        let out: [u8; 32] = hasher.finalize().into();

        CertFreeCertificateContext(cert);
        CryptMsgClose(msg);
        CertCloseStore(store, 0);
        Some(out)
    }
}

#[cfg(not(windows))]
fn signature(_blob: &[u8]) -> Option<[u8; 32]> {
    None
}

fn certificate_blob(data: &[u8], opt: usize) -> Option<&[u8]> {
    let directories = u32::from_le_bytes(data[opt + 108..opt + 112].try_into().ok()?);
    if directories < 5 {
        return None;
    }

    let entry = opt + 112 + 4 * 8;
    let offset = u32::from_le_bytes(data[entry..entry + 4].try_into().ok()?) as usize;
    let size = u32::from_le_bytes(data[entry + 4..entry + 8].try_into().ok()?) as usize;
    if size < 8 || offset.checked_add(size)? > data.len() {
        return None;
    }

    let length = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
    let start = offset + 8;
    let end = start.checked_add(length.checked_sub(8)?)?.min(data.len());
    Some(&data[start..end])
}

pub fn from_file(path: &Path, machine_guid: &str, user_id: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let file_size = data.len() as u64;
    let window = &data[..data.len().min(PREFIX)];

    if window.len() < 0x40 || u16::from_le_bytes([window[0], window[1]]) != 0x5A4D {
        return None;
    }

    let pe = u32::from_le_bytes([window[0x3C], window[0x3D], window[0x3E], window[0x3F]]) as usize;
    if pe + 24 > window.len() || &window[pe..pe + 4] != b"PE\0\0" {
        return None;
    }

    let header = pe + 4;
    let sections = u16::from_le_bytes([window[header + 2], window[header + 3]]) as usize;
    let timestamp = u32::from_le_bytes([
        window[header + 4],
        window[header + 5],
        window[header + 6],
        window[header + 7],
    ]);
    let opt_size = u16::from_le_bytes([window[header + 16], window[header + 17]]) as usize;
    let opt = header + 20;

    if u16::from_le_bytes([window[opt], window[opt + 1]]) != 0x20B {
        return None;
    }

    let size_of_image = u32::from_le_bytes([
        window[opt + 56],
        window[opt + 57],
        window[opt + 58],
        window[opt + 59],
    ]);

    let table = opt + opt_size;
    let table_end = table + sections * 40;
    if table_end > window.len() {
        return None;
    }

    let header_hash = sha(&window[..table]);
    let section_hash = sha(&window[table..table_end]);

    let cert = certificate_blob(&data, opt).and_then(signature);
    let flags = 2 | u8::from(cert.is_some());

    let out = payload(
        flags,
        file_size,
        timestamp,
        size_of_image,
        header_hash,
        machine_user(machine_guid, user_id),
        section_hash,
        cert.unwrap_or([0u8; 32]),
    );

    Some(xor(out, user_id))
}
