# Runs cargo with the MSVC toolchain on PATH.
#
# The Build Tools for Visual Studio are installed on this machine but are not on
# PATH, so a bare `cargo test` fails with "linker `link.exe` not found" (or, from
# Git Bash, resolves to the coreutils `link` and fails with "extra operand").
# Sourcing vcvars64 once per invocation is the least surprising fix: it is what
# the MSVC target expects, and it leaves the Android cross builds alone since
# those go through the NDK's clang.
#
# Usage:  .\tools\cargo.ps1 test
#         .\tools\cargo.ps1 clippy --all-targets

$ErrorActionPreference = 'Stop'

$vcvars = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path $vcvars)) {
    throw "vcvars64.bat not found at $vcvars - install the 'Desktop development with C++' workload, or set up the MSVC environment yourself before calling cargo."
}

# Import the environment vcvars sets (PATH, LIB, INCLUDE) into this session.
# Some launchers leave both `PATH` and `Path` in the native environment. `cmd set`
# prints both, and importing the shorter stale value last would silently remove MSVC.
$vcvarsPath = $null
cmd /c "call `"$vcvars`" >NUL 2>&1 && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        $name = $Matches[1]
        $value = $Matches[2]
        if ($name -ieq 'PATH') {
            if ($null -eq $vcvarsPath -or $value.Length -gt $vcvarsPath.Length) {
                $vcvarsPath = $value
            }
        } else {
            Set-Item -Path "env:$name" -Value $value
        }
    }
}
if ($null -eq $vcvarsPath) {
    throw 'vcvars64.bat did not return PATH'
}
$env:Path = $vcvarsPath

& cargo @args
exit $LASTEXITCODE
