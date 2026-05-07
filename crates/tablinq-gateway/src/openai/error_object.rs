use serde_json::{Value, json};

pub fn openai_error(status: u16, error_type: &str, code: &str, message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": Value::Null,
            "code": code,
        },
        "status": status,
    })
}
