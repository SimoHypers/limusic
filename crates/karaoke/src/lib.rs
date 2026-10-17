pub mod board;
pub mod cache;
pub mod cache_codec;
pub mod device;
pub mod download;
#[cfg(test)]
mod flac_read;
pub mod jobs;
pub mod mdx;
pub mod models;
pub mod pipeline;
pub mod runtime;
pub mod stft;

/// Helper to sanitize error messages (removes URLs and absolute paths).
pub fn sanitize_error(err: &str) -> String {
    let mut sanitized = err.to_string();

    while let Some(start) = sanitized.find("http://").or_else(|| sanitized.find("https://")) {
        let end = sanitized[start..]
            .find(|c: char| {
                c.is_whitespace()
                    || c == '"'
                    || c == '\''
                    || c == ')'
                    || c == ']'
                    || c == ','
                    || c == ';'
            })
            .map(|e| start + e)
            .unwrap_or(sanitized.len());
        sanitized.replace_range(start..end, "[url]");
    }

    let mut i = 0;
    while i < sanitized.len() {
        let bytes = sanitized.as_bytes();
        let is_win_drive = i + 2 < bytes.len()
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/');

        let is_unc = i + 1 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'\\';

        let is_unix_path =
            bytes[i] == b'/' && (i + 1 < bytes.len() && has_path_structure(&sanitized[i..]));

        if is_win_drive || is_unc || is_unix_path {
            let rest = &sanitized[i..];
            let len = rest
                .find(|c: char| {
                    c.is_whitespace()
                        || c == '"'
                        || c == '\''
                        || c == ')'
                        || c == ']'
                        || c == ','
                        || c == ';'
                })
                .unwrap_or(rest.len());

            let path_str = &rest[..len];
            let valid = if is_unix_path {
                path_str.contains('/') && path_str.len() > 1 && path_str != "/" && path_str != "//"
            } else {
                true
            };

            if valid {
                sanitized.replace_range(i..i + len, "[path]");
                i += 6;
                continue;
            }
        }
        i += 1;
    }

    sanitized
}

fn has_path_structure(s: &str) -> bool {
    let token_len = s
        .find(|c: char| {
            c.is_whitespace()
                || c == '"'
                || c == '\''
                || c == ')'
                || c == ']'
                || c == ','
                || c == ';'
        })
        .unwrap_or(s.len());
    let token = &s[..token_len];
    token.matches('/').count() >= 2 || token.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_error() {
        let msg = "Error processing C:\\Users\\maksi\\Desktop\\song.mp3 and /home/user/music/test.wav from https://example.com/stream?token=abc&sig=xyz";
        let sanitized = sanitize_error(msg);
        assert!(!sanitized.contains("C:\\Users"), "Windows path should be removed");
        assert!(!sanitized.contains("/home/user"), "Unix path should be removed");
        assert!(
            !sanitized.contains("https://example.com"),
            "URL with query parameters should be removed"
        );
        assert!(!sanitized.contains("token=abc"), "Query parameters should be removed");
        assert!(sanitized.contains("[path]"), "Should contain [path]");
        assert!(sanitized.contains("[url]"), "Should contain [url]");
    }
}
