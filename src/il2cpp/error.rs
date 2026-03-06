use std::str::Utf8Error;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Il2CppError {
  #[error(transparent)]
  Utf8(#[from] Utf8Error),

  #[error("file not found {0}")]
  FileNotFound(&'static str),
  #[error("function not found {0}")]
  FunctionNotFound(&'static str),
  #[error("root path not found")]
  RootNotFound,
  #[error("function returned null {0}")]
  ReturnedNull(&'static str)
}