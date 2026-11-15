/// Format bytes as KB
pub fn format_bytes(bytes: usize) -> String {
    format!("{:.1} KB", bytes as f64 / 1024.0)
}
