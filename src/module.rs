use std::sync::LazyLock;

use windows::{core::w, Win32::System::LibraryLoader::GetModuleHandleW};

pub static GAMEASSEMBLY_HANDLE: LazyLock<usize> = LazyLock::new(|| unsafe {
  GetModuleHandleW(w!("GameAssembly"))
      .expect("GameAssembly was not found in the game process")
      .0 as usize
});

pub static UNITYPLAYER_HANDLE: LazyLock<usize> = LazyLock::new(|| unsafe {
  GetModuleHandleW(w!("UnityPlayer"))
      .expect("UnityPlayer was not found in the game process")
      .0 as usize
});
