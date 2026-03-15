use anyhow::Error;

pub mod config;
pub mod models;
pub mod synology;

pub fn format_error_chain(error: &Error) -> String {
    error
        .chain()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_full_error_chain() {
        let error = anyhow::anyhow!("root cause").context("higher level");

        assert_eq!(format_error_chain(&error), "higher level: root cause");
    }
}
