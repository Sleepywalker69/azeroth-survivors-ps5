@echo off
rem Runs cargo with the MSVC tools on PATH (link.exe and the SDK's libraries), so a shell whose
rem PATH has another link.exe first (Git Bash's) still links with Microsoft's.
rem   tools\cargo-msvc.cmd build --release --manifest-path rust\Cargo.toml
setlocal
set "VCVARS="
for %%v in (
  "%ProgramFiles%\Microsoft Visual Studio\18\Insiders\VC\Auxiliary\Build\vcvars64.bat"
  "%ProgramFiles%\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"
  "%ProgramFiles%\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
  "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
) do if not defined VCVARS if exist %%v set "VCVARS=%%~v"
if not defined VCVARS (
  echo cargo-msvc: no Visual Studio with the C++ tools was found 1>&2
  exit /b 2
)
call "%VCVARS%" >nul || exit /b 1
cargo %*
