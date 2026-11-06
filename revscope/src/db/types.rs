use compact_str::CompactString;
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use smallvec::SmallVec;

/// SQLite text column decoded directly into a compact string without an intermediate `String`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SqlCompactStr(CompactString);

impl SqlCompactStr {
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq<&str> for SqlCompactStr {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl FromSql for SqlCompactStr {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Ok(Self(CompactString::new(text)))
    }
}

impl From<SqlCompactStr> for CompactString {
    fn from(value: SqlCompactStr) -> Self {
        value.0
    }
}

/// Client-generated page id from the tracker (`pg`), stored as a 12-byte blob.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ClientPageId(SmallVec<[u8; 12]>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvalidClientPageId;

impl ClientPageId {
    pub(crate) const HEX_LEN: usize = 24;

    pub(crate) fn from_hex(value: &str) -> Result<Self, InvalidClientPageId> {
        if !is_lower_hex(value, Self::HEX_LEN) {
            return Err(InvalidClientPageId);
        }
        let mut output = SmallVec::<[u8; 12]>::new();
        let bytes = value.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let high = crate::util::hex_nibble(bytes[index]).ok_or(InvalidClientPageId)?;
            let low = crate::util::hex_nibble(bytes[index + 1]).ok_or(InvalidClientPageId)?;
            output.push((high << 4) | low);
            index += 2;
        }
        Ok(Self(output))
    }

    pub(crate) fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, InvalidClientPageId> {
        let bytes = bytes.as_ref();
        if bytes.len() != 12 {
            return Err(InvalidClientPageId);
        }
        Ok(Self(SmallVec::from_slice(bytes)))
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for ClientPageId {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl FromSql for ClientPageId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let ValueRef::Blob(bytes) = value else {
            return Err(rusqlite::types::FromSqlError::InvalidType);
        };
        Self::from_bytes(bytes).map_err(|_| rusqlite::types::FromSqlError::InvalidType)
    }
}

impl ToSql for ClientPageId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_slice()))
    }
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
