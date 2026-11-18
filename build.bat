@echo off
cd /d "%~dp0"
where cargo >nul 2>nul || (
  echo Rust not found. Install: https://rustup.rs  ^(then reopen this window^)
  pause
  exit /b 1
)
cargo build --release || (pause & exit /b 1)
copy /y target\release\assume.exe assume.exe >nul
echo.
echo OK: %~dp0assume.exe
pause
