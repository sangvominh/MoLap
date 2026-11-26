@echo off
setlocal
set "CARGO=cargo.exe"
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if defined MOLAP_CARGO set "CARGO=%MOLAP_CARGO%"
pushd "%~dp0" || exit /b 1
set "CARGO_TARGET_DIR=%CD%\target"
"%CARGO%" build --release --locked
if errorlevel 1 (
    popd
    exit /b 1
)
if exist "MoLap.exe" (
    fc /B "target\release\molap.exe" "MoLap.exe" >nul
    if not errorlevel 1 (
        popd
        exit /b 0
    )
)
copy /Y "target\release\molap.exe" "MoLap.exe" >nul
if errorlevel 1 (
    echo Could not replace MoLap.exe. Exit the running copy and try again.
    popd
    exit /b 1
)
popd
exit /b 0
