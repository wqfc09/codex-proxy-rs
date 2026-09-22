//! 身份用例共享的用户名、密码校验与哈希。

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::model::AdminError;

const MIN_PASSWORD_BYTES: usize = 12;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_USERNAME_BYTES: usize = 128;

pub(super) fn validate_username(username: &str) -> Result<(), AdminError> {
    if username.is_empty()
        || username.len() > MAX_USERNAME_BYTES
        || username.trim() != username
        || username.chars().any(char::is_control)
    {
        return Err(AdminError::invalid("用户名格式不合法"));
    }
    Ok(())
}

pub(super) fn validate_password(password: &str) -> Result<(), AdminError> {
    let trimmed = password.trim();
    if trimmed.chars().count() < MIN_PASSWORD_BYTES
        || password.len() > MAX_PASSWORD_BYTES
        || password.chars().any(char::is_control)
        || crate::WEAK_ADMIN_PASSWORDS.contains(&trimmed.to_ascii_lowercase().as_str())
    {
        return Err(AdminError::invalid(
            "密码至少需要 12 个字符，最多 1024 字节，不能使用常见弱口令或控制字符",
        ));
    }
    Ok(())
}

pub(super) fn hash_password(password: &str) -> Result<String, AdminError> {
    validate_password(password)?;
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| AdminError::internal("密码哈希失败"))
}

pub(super) fn verify_password(password: &str, encoded: &str) -> Result<bool, AdminError> {
    let hash =
        PasswordHash::new(encoded).map_err(|_| AdminError::internal("已保存的密码哈希不合法"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok())
}
