# Scout's terminal panel. Dot-sourced after the user's profile, in the panel's shell only: the agent runs its commands
# elsewhere and keeps PowerShell's standard errors. Two things change here:
#   - Errors print as one short line instead of PowerShell's five-line red block.
#   - A Linux command typed at the prompt is rewritten to its PowerShell equivalent before it runs, but only when the
#     rewrite is certain: the command isn't a real program here, every flag is understood, and the line has no syntax
#     this can't read. Anything else runs exactly as typed. The rewritten line stays in the prompt, so it's visible.
# Works in Windows PowerShell 5.1 (no `e, ternaries or ??) and PowerShell 7.

$global:__scoutIsPs7 = $PSVersionTable.PSVersion.Major -ge 7

# ---------- errors ----------

function global:__ScoutErrorLine($e) {
    $esc = [char]27
    $red = "$esc[38;2;232;100;100m"
    $dim = "$esc[38;2;122;122;122m"
    $off = "$esc[0m"
    $id = "$($e.FullyQualifiedErrorId)"
    $msg = $null
    if ($e.ErrorDetails -and $e.ErrorDetails.Message) { $msg = $e.ErrorDetails.Message } else { $msg = "$($e.Exception.Message)" }

    # A program's own stderr, captured with 2>&1: its words, as they are.
    if ($id -like 'NativeCommandError*') { return "$red$msg$off" }

    if ($e.Exception -is [System.Management.Automation.CommandNotFoundException]) {
        $name = "$($e.TargetObject)"
        $msg = "'$name' isn't a command, program or script here."
        if (Test-Path -LiteralPath $name -ErrorAction SilentlyContinue) { $msg = "'$name' isn't a command. To run the file in this folder, type .\$name" }
    } elseif ($e.Exception -is [System.Management.Automation.ParseException] -and $e.Exception.Errors -and $e.Exception.Errors.Count) {
        $msg = $e.Exception.Errors[0].Message
    }
    $msg = $msg -replace '\s*Check the spelling of the name, or if a path was included, verify that the path is correct and try again\.?', ''
    $msg = ($msg -replace '\s+', ' ').Trim()

    $where = ''
    $inv = $e.InvocationInfo
    if ($inv -and $inv.ScriptName) { $where = " $dim(" + (Split-Path -Leaf $inv.ScriptName) + ":$($inv.ScriptLineNumber))$off" }
    return "$red$msg$off$where"
}

# Interactive output goes through Out-Default, errors included, so a proxy of it can print them its own way. Everything
# else passes through to the real Out-Default untouched. $Error still holds the full records.
function global:Out-Default {
    [CmdletBinding()]
    param([switch]$Transcript, [Parameter(ValueFromPipeline = $true)][psobject]$InputObject)
    begin {
        $real = $ExecutionContext.InvokeCommand.GetCommand('Microsoft.PowerShell.Core\Out-Default', [System.Management.Automation.CommandTypes]::Cmdlet)
        if ($PSBoundParameters.ContainsKey('InputObject')) { [void]$PSBoundParameters.Remove('InputObject') }
        $pipe = { & $real @PSBoundParameters }.GetSteppablePipeline($MyInvocation.CommandOrigin)
        $pipe.Begin($PSCmdlet)
    }
    process {
        $o = $InputObject
        if ($o -is [System.Management.Automation.ErrorRecord]) {
            $text = $null
            try { $text = __ScoutErrorLine $o } catch {}
            if ($text) { $Host.UI.WriteLine($text) } else { $pipe.Process($o) }
        } else {
            $pipe.Process($o)
        }
    }
    end { $pipe.End() }
}

# ---------- Linux commands ----------

# Splits a bash-style line into words and operators. $null when the line holds anything that would need reading as
# bash or as PowerShell to be sure of ($ ` ( ) { } < > # @ , and so on): those lines are left alone.
function global:__ScoutTokens([string]$s) {
    $out = New-Object System.Collections.ArrayList
    $sb = New-Object System.Text.StringBuilder
    $has = $false
    $quoted = $false
    $i = 0
    $n = $s.Length
    while ($i -lt $n) {
        $c = $s[$i]
        if ($c -eq "'") {
            $j = $s.IndexOf("'", $i + 1)
            if ($j -lt 0) { return $null }
            [void]$sb.Append($s.Substring($i + 1, $j - $i - 1)); $has = $true; $quoted = $true; $i = $j + 1; continue
        }
        if ($c -eq '"') {
            $j = $s.IndexOf('"', $i + 1)
            if ($j -lt 0) { return $null }
            $inner = $s.Substring($i + 1, $j - $i - 1)
            if ($inner -match '[\$`\\]') { return $null }
            [void]$sb.Append($inner); $has = $true; $quoted = $true; $i = $j + 1; continue
        }
        if ($c -eq ' ' -or $c -eq "`t") {
            if ($has) { [void]$out.Add(@{ t = $sb.ToString(); q = $quoted }); [void]$sb.Clear(); $has = $false; $quoted = $false }
            $i++; continue
        }
        $op = $null
        if ($c -eq '|') { if ($i + 1 -lt $n -and $s[$i + 1] -eq '|') { $op = '||' } else { $op = '|' } }
        elseif ($c -eq '&') { if ($i + 1 -lt $n -and $s[$i + 1] -eq '&') { $op = '&&' } else { return $null } }
        elseif ($c -eq ';') { $op = ';' }
        if ($op) {
            if ($has) { [void]$out.Add(@{ t = $sb.ToString(); q = $quoted }); [void]$sb.Clear(); $has = $false; $quoted = $false }
            [void]$out.Add(@{ op = $op }); $i += $op.Length; continue
        }
        if ('$`(){}<>#@,"' + "'" -match [regex]::Escape([string]$c) -or [int][char]$c -lt 32) { return $null }
        [void]$sb.Append($c); $has = $true; $i++
    }
    if ($has) { [void]$out.Add(@{ t = $sb.ToString(); q = $quoted }) }
    return , $out
}

# A word as PowerShell should read it: bare when that's safe, else single-quoted.
function global:__ScoutQ([string]$w) {
    if ($w -match '^[A-Za-z0-9_\.\-/\\:~\*\?\[\]\+=%]+$' -and $w -notmatch '^[-+]') { return $w }
    return "'" + ($w -replace "'", "''") + "'"
}
function global:__ScoutList($words) { return (($words | ForEach-Object { __ScoutQ $_ }) -join ',') }

# Reads a command's flags. $null when one isn't in $letters/$long (or a value flag has no value), so the caller gives up.
function global:__ScoutFlags($words, [string]$letters, $long = @{}, [string]$valued = '') {
    # Case matters: grep -e and -E, ls -s and -S.
    $flags = New-Object System.Collections.Hashtable
    $rest = New-Object System.Collections.ArrayList
    $i = 0
    while ($i -lt $words.Count) {
        $w = $words[$i]
        if (-not $w.q -and $w.t -match '^--[a-z]') {
            $name = $w.t.Substring(2)
            if (-not $long.ContainsKey($name)) { return $null }
            $flags[$long[$name]] = $true
        } elseif (-not $w.q -and $w.t -match '^-[A-Za-z0-9]') {
            $k = 1
            while ($k -lt $w.t.Length) {
                $f = [string]$w.t[$k]
                if ($valued.Contains($f)) {
                    $v = $w.t.Substring($k + 1)
                    if (-not $v) { $i++; if ($i -ge $words.Count) { return $null }; $v = $words[$i].t }
                    $flags[$f] = $v; break
                }
                if ($f -match '[0-9]' -and $valued.Contains('#')) { $flags['#'] = $w.t.Substring($k); break }
                if (-not $letters.Contains($f)) { return $null }
                $flags[$f] = $true; $k++
            }
        } elseif (-not $w.q -and $w.t -eq '--') {
            return $null
        } else {
            [void]$rest.Add($w.t)
        }
        $i++
    }
    return @{ f = $flags; rest = $rest }
}

function global:__ScoutHas($p, [string]$letters) { foreach ($c in $letters.ToCharArray()) { if ($p.f.ContainsKey([string]$c)) { return $true } }; return $false }

# Each takes the words after the command name and whether input is piped in, and returns PowerShell, $false when the
# command already works as typed, or $null when the rewrite wouldn't be certain.
$global:__scoutRules = @{
    ls = {
        param($w, $piped)
        $p = __ScoutFlags $w 'laAh1RtrSG' @{ all = 'a'; 'almost-all' = 'A'; recursive = 'R'; 'human-readable' = 'h' }
        if (-not $p) { return $null }
        if ($p.f.Count -eq 0 -and $p.rest.Count -le 1) { return $false }
        $s = 'Get-ChildItem'
        if ($p.rest.Count) { $s += ' -Path ' + (__ScoutList $p.rest) }
        if (__ScoutHas $p 'aA') { $s += ' -Force' }
        if (__ScoutHas $p 'R') { $s += ' -Recurse' }
        if ((__ScoutHas $p '1') -and -not (__ScoutHas $p 'l')) { $s += ' -Name' }
        $desc = -not (__ScoutHas $p 'r')
        if (__ScoutHas $p 't') { $s += ' | Sort-Object LastWriteTime' + $(if ($desc) { ' -Descending' } else { '' }) }
        elseif (__ScoutHas $p 'S') { $s += ' | Sort-Object Length' + $(if ($desc) { ' -Descending' } else { '' }) }
        elseif (__ScoutHas $p 'r') { $s += ' | Sort-Object Name -Descending' }
        return $s
    }
    ll = { param($w, $piped); & $__scoutRules.ls (@(@{ t = '-l'; q = $false }) + $w) $piped }
    la = { param($w, $piped); & $__scoutRules.ls (@(@{ t = '-la'; q = $false }) + $w) $piped }
    rm = {
        param($w, $piped)
        $p = __ScoutFlags $w 'rRfvd' @{ recursive = 'r'; force = 'f'; verbose = 'v' }
        if (-not $p -or $p.rest.Count -eq 0) { return $null }
        if ($p.f.Count -eq 0 -and $p.rest.Count -eq 1) { return $false }
        $s = 'Remove-Item -Path ' + (__ScoutList $p.rest)
        if (__ScoutHas $p 'rR') { $s += ' -Recurse' }
        if (__ScoutHas $p 'f') { $s += ' -Force -ErrorAction SilentlyContinue' }
        if (__ScoutHas $p 'v') { $s += ' -Verbose' }
        return $s
    }
    cp = {
        param($w, $piped)
        $p = __ScoutFlags $w 'rRafv' @{ recursive = 'r'; force = 'f'; verbose = 'v' }
        if (-not $p -or $p.rest.Count -lt 2) { return $null }
        if ($p.f.Count -eq 0 -and $p.rest.Count -eq 2) { return $false }
        $src = $p.rest[0..($p.rest.Count - 2)]
        $s = 'Copy-Item -Path ' + (__ScoutList $src) + ' -Destination ' + (__ScoutQ $p.rest[$p.rest.Count - 1])
        if (__ScoutHas $p 'rRa') { $s += ' -Recurse' }
        if (__ScoutHas $p 'f') { $s += ' -Force' }
        if (__ScoutHas $p 'v') { $s += ' -Verbose' }
        return $s
    }
    mv = {
        param($w, $piped)
        $p = __ScoutFlags $w 'fv' @{ force = 'f'; verbose = 'v' }
        if (-not $p -or $p.rest.Count -lt 2) { return $null }
        if ($p.f.Count -eq 0 -and $p.rest.Count -eq 2) { return $false }
        $src = $p.rest[0..($p.rest.Count - 2)]
        $s = 'Move-Item -Path ' + (__ScoutList $src) + ' -Destination ' + (__ScoutQ $p.rest[$p.rest.Count - 1])
        if (__ScoutHas $p 'f') { $s += ' -Force' }
        if (__ScoutHas $p 'v') { $s += ' -Verbose' }
        return $s
    }
    mkdir = {
        param($w, $piped)
        $p = __ScoutFlags $w 'pv' @{ parents = 'p'; verbose = 'v' }
        if (-not $p -or $p.rest.Count -eq 0) { return $null }
        if ($p.f.Count -eq 0 -and $p.rest.Count -eq 1) { return $false }
        $s = 'New-Item -ItemType Directory -Path ' + (__ScoutList $p.rest)
        if (__ScoutHas $p 'p') { $s += ' -Force' }
        return $s + ' | Out-Null'
    }
    cat = {
        param($w, $piped)
        $p = __ScoutFlags $w ''
        if (-not $p -or $p.rest.Count -eq 0) { return $null }
        if ($p.rest.Count -eq 1) { return $false }
        return 'Get-Content -Path ' + (__ScoutList $p.rest)
    }
    touch = {
        param($w, $piped)
        $p = __ScoutFlags $w ''
        if (-not $p -or $p.rest.Count -eq 0) { return $null }
        $parts = foreach ($f in $p.rest) {
            if ($f -match '[\*\?\[]') { return $null }
            if (Test-Path -LiteralPath $f) { '(Get-Item -LiteralPath ' + (__ScoutQ $f) + ').LastWriteTime = Get-Date' }
            else { 'New-Item -ItemType File -Path ' + (__ScoutQ $f) + ' | Out-Null' }
        }
        return ($parts -join '; ')
    }
    grep = {
        param($w, $piped)
        $p = __ScoutFlags $w 'ivnrREFlcwHs' @{ 'ignore-case' = 'i'; 'invert-match' = 'v'; recursive = 'r'; 'files-with-matches' = 'l'; count = 'c'; 'word-regexp' = 'w'; 'line-number' = 'n'; 'extended-regexp' = 'E'; 'fixed-strings' = 'F' } 'e'
        if (-not $p) { return $null }
        $rest = @($p.rest)
        $pat = $p.f['e']
        if (-not $pat) { if ($rest.Count -eq 0) { return $null }; $pat = $rest[0]; $rest = @($rest | Select-Object -Skip 1) }
        $fixed = __ScoutHas $p 'F'
        # Plain grep reads + ? | ( ) { } literally, .NET regex doesn't: only certain without them.
        if (-not $fixed -and -not (__ScoutHas $p 'E') -and $pat -match '[\+\?\|\(\)\{\}]') { return $null }
        if (__ScoutHas $p 'w') { if ($fixed) { $pat = '\b' + [regex]::Escape($pat) + '\b'; $fixed = $false } else { $pat = '\b(?:' + $pat + ')\b' } }
        $opts = ' -Pattern ' + (__ScoutQ $pat)
        if ($fixed) { $opts += ' -SimpleMatch' }
        if (-not (__ScoutHas $p 'i')) { $opts += ' -CaseSensitive' }
        if (__ScoutHas $p 'v') { $opts += ' -NotMatch' }
        $tail = ''
        if (__ScoutHas $p 'l') { $tail = ' | Select-Object -ExpandProperty Path -Unique' }
        if (__ScoutHas $p 'c') { if ($rest.Count -gt 1 -or (__ScoutHas $p 'rR')) { return $null }; $tail = ' | Measure-Object | Select-Object -ExpandProperty Count' }
        $err = ''
        if (__ScoutHas $p 's') { $err = ' -ErrorAction SilentlyContinue' }
        if (__ScoutHas $p 'rR') {
            $dirs = $rest; if ($dirs.Count -eq 0) { $dirs = @('.') }
            return 'Get-ChildItem -Path ' + (__ScoutList $dirs) + ' -Recurse -File' + $err + ' | Select-String' + $opts + $tail
        }
        if ($rest.Count) { return 'Select-String' + $opts + ' -Path ' + (__ScoutList $rest) + $err + $tail }
        if ($piped -and -not (__ScoutHas $p 'n')) { return 'Out-String -Stream | Select-String' + $opts + $tail }
        return $null
    }
    which = {
        param($w, $piped)
        $p = __ScoutFlags $w 'a'
        if (-not $p -or $p.rest.Count -eq 0) { return $null }
        $all = ''; if (__ScoutHas $p 'a') { $all = ' -All' }
        if ($p.rest.Count -eq 1) { return '(Get-Command ' + (__ScoutQ $p.rest[0]) + $all + ').Source' }
        return 'Get-Command ' + (__ScoutList $p.rest) + $all + ' | Format-Table Name, Source'
    }
    head = {
        param($w, $piped)
        $p = __ScoutFlags $w '' @{} 'n#'
        if (-not $p -or $p.rest.Count -gt 1) { return $null }
        $k = 10; if ($p.f['n']) { $k = $p.f['n'] } elseif ($p.f['#']) { $k = $p.f['#'] }
        if ("$k" -notmatch '^\d+$') { return $null }
        if ($p.rest.Count) { return 'Get-Content -Path ' + (__ScoutQ $p.rest[0]) + " -TotalCount $k" }
        if ($piped) { return "Select-Object -First $k" }
        return $null
    }
    tail = {
        param($w, $piped)
        $p = __ScoutFlags $w 'fF' @{ follow = 'f' } 'n#'
        if (-not $p -or $p.rest.Count -gt 1) { return $null }
        $k = 10; if ($p.f['n']) { $k = $p.f['n'] } elseif ($p.f['#']) { $k = $p.f['#'] }
        if ("$k" -notmatch '^\d+$') { return $null }
        if ($p.rest.Count) { $s = 'Get-Content -Path ' + (__ScoutQ $p.rest[0]) + " -Tail $k"; if (__ScoutHas $p 'fF') { $s += ' -Wait' }; return $s }
        if ($piped -and -not (__ScoutHas $p 'fF')) { return "Select-Object -Last $k" }
        return $null
    }
    wc = {
        param($w, $piped)
        $p = __ScoutFlags $w 'lwcm' @{ lines = 'l'; words = 'w'; bytes = 'c'; chars = 'm' }
        if (-not $p -or $p.f.Count -ne 1 -or $p.rest.Count -gt 1) { return $null }
        $what = @{ l = 'Lines'; w = 'Words'; c = 'Characters'; m = 'Characters' }[[string]@($p.f.Keys)[0]]
        $switch = @{ Lines = '-Line'; Words = '-Word'; Characters = '-Character' }[$what]
        if ($p.rest.Count) { return '(Get-Content -Path ' + (__ScoutQ $p.rest[0]) + " | Measure-Object $switch).$what" }
        if (-not $piped) { return $null }
        if ($what -eq 'Lines') { return 'Measure-Object | Select-Object -ExpandProperty Count' }
        return "Out-String | Measure-Object $switch | Select-Object -ExpandProperty $what"
    }
    export = {
        param($w, $piped)
        if ($w.Count -eq 0) { return 'Get-ChildItem Env:' }
        $parts = foreach ($x in $w) {
            if ($x.t -notmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') { return $null }
            '$env:' + $Matches[1] + ' = ' + "'" + ($Matches[2] -replace "'", "''") + "'"
        }
        return ($parts -join '; ')
    }
    unset = {
        param($w, $piped)
        if ($w.Count -eq 0) { return $null }
        $parts = foreach ($x in $w) { if ($x.t -notmatch '^[A-Za-z_][A-Za-z0-9_]*$') { return $null }; 'Remove-Item Env:' + $x.t + ' -ErrorAction SilentlyContinue' }
        return ($parts -join '; ')
    }
    env = { param($w, $piped); if ($w.Count) { return $null }; return 'Get-ChildItem Env:' }
    printenv = {
        param($w, $piped)
        if ($w.Count -eq 0) { return 'Get-ChildItem Env:' }
        if ($w.Count -eq 1 -and $w[0].t -match '^[A-Za-z_][A-Za-z0-9_]*$') { return '$env:' + $w[0].t }
        return $null
    }
    kill = {
        param($w, $piped)
        $force = $false; $ids = @()
        foreach ($x in $w) {
            if ($x.t -in @('-9', '-KILL', '-SIGKILL')) { $force = $true }
            elseif ($x.t -in @('-15', '-TERM', '-SIGTERM')) { }
            elseif ($x.t -match '^\d+$') { $ids += $x.t }
            else { return $null }
        }
        if ($ids.Count -eq 0) { return $null }
        if (-not $force -and $ids.Count -eq 1 -and $w.Count -eq 1) { return $false }
        return 'Stop-Process -Id ' + ($ids -join ',') + $(if ($force) { ' -Force' } else { '' })
    }
    killall = {
        param($w, $piped)
        $force = $false; $names = @()
        foreach ($x in $w) { if ($x.t -eq '-9') { $force = $true } elseif ($x.t -match '^[\w\.\-]+$') { $names += ($x.t -replace '\.exe$', '') } else { return $null } }
        if ($names.Count -eq 0) { return $null }
        return 'Stop-Process -Name ' + ($names -join ',') + $(if ($force) { ' -Force' } else { '' })
    }
    ps = {
        param($w, $piped)
        if ($w.Count -eq 0) { return $false }
        if ($w.Count -eq 1 -and $w[0].t -in @('aux', '-aux', '-ef', '-e', '-A', 'ax', '-eF')) { return 'Get-Process' }
        return $null
    }
    open = { param($w, $piped); if ($w.Count -ne 1 -or $w[0].t -match '^-') { return $null }; return 'Invoke-Item ' + (__ScoutQ $w[0].t) }
    ln = {
        param($w, $piped)
        $p = __ScoutFlags $w 'sf' @{ symbolic = 's'; force = 'f' }
        if (-not $p -or $p.rest.Count -ne 2) { return $null }
        $type = 'HardLink'; if (__ScoutHas $p 's') { $type = 'SymbolicLink' }
        $s = "New-Item -ItemType $type -Path " + (__ScoutQ $p.rest[1]) + ' -Target ' + (__ScoutQ $p.rest[0])
        if (__ScoutHas $p 'f') { $s += ' -Force' }
        return $s
    }
    ifconfig = { param($w, $piped); if ($w.Count) { return $null }; return 'ipconfig' }
    ip = { param($w, $piped); if ($w.Count -eq 1 -and $w[0].t -in @('a', 'addr', 'address')) { return 'ipconfig' }; return $null }
    df = { param($w, $piped); if ($w.Count -gt 1 -or ($w.Count -eq 1 -and $w[0].t -ne '-h')) { return $null }; return 'Get-PSDrive -PSProvider FileSystem' }
    less = {
        param($w, $piped)
        if ($w.Count -eq 0 -and $piped) { return 'Out-Host -Paging' }
        if ($w.Count -eq 1 -and -not ($w[0].t -match '^-')) { return 'Get-Content -Path ' + (__ScoutQ $w[0].t) + ' | Out-Host -Paging' }
        return $null
    }
    nproc = { param($w, $piped); if ($w.Count) { return $null }; return '[Environment]::ProcessorCount' }
    find = {
        param($w, $piped)
        $path = '.'; $s = ''; $i = 0
        if ($w.Count -and $w[0].t -notmatch '^-') { $path = $w[0].t; $i = 1 }
        while ($i -lt $w.Count) {
            $k = $w[$i].t; $v = $null; if ($i + 1 -lt $w.Count) { $v = $w[$i + 1].t }
            if ($null -eq $v) { return $null }
            switch -CaseSensitive ($k) {
                '-name' { $s += ' -Filter ' + (__ScoutQ $v) }
                '-iname' { $s += ' -Filter ' + (__ScoutQ $v) }
                '-type' { if ($v -eq 'f') { $s += ' -File' } elseif ($v -eq 'd') { $s += ' -Directory' } else { return $null } }
                '-maxdepth' { if ($v -notmatch '^[1-9]\d*$') { return $null }; $s += ' -Depth ' + ([int]$v - 1) }
                default { return $null }
            }
            $i += 2
        }
        return 'Get-ChildItem -Path ' + (__ScoutQ $path) + ' -Recurse' + $s + ' -Name'
    }
}
$global:__scoutRules.pkill = $global:__scoutRules.killall
$global:__scoutRules['xdg-open'] = $global:__scoutRules.open

# What each built-in alias or function must still point at for its rule to apply (a profile may have changed it).
$global:__scoutLookups = @{}
$global:__scoutBuiltin = @{ ls = 'Get-ChildItem'; rm = 'Remove-Item'; cp = 'Copy-Item'; mv = 'Move-Item'; cat = 'Get-Content'; kill = 'Stop-Process'; ps = 'Get-Process'; mkdir = 'New-Item' }

function global:__ScoutSegment($words, [bool]$piped) {
    $name = $words[0].t
    if ($words[0].q) { return $false }
    $rule = $__scoutRules[$name]
    if (-not $rule) { return $false }
    # Looked up once per name and PATH: Get-Command searches PATH for a missing name, which is the slow part.
    $key = "$name|$env:PATH"
    # Aliases and functions straight from their drives: Get-Command would also search every module, ~40 ms.
    $cmd = $null
    if (Test-Path -LiteralPath "Alias:$name") { $cmd = Get-Item -LiteralPath "Alias:$name" }
    elseif (Test-Path -LiteralPath "Function:$name") { $cmd = Get-Item -LiteralPath "Function:$name" }
    if (-not $cmd) {
        if (-not $__scoutLookups.ContainsKey($key)) { $__scoutLookups[$key] = @(Get-Command -Name $name -CommandType Application, ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1) }
        $cmd = $__scoutLookups[$key] | Select-Object -First 1
    }
    if ($cmd) {
        $want = $__scoutBuiltin[$name]
        $ok = $false
        if ($want -and $cmd.CommandType -eq 'Alias' -and $cmd.Definition -eq $want) { $ok = $true }
        if ($want -and $cmd.CommandType -eq 'Function' -and $cmd.Definition -match [regex]::Escape($want)) { $ok = $true }
        # Windows' own find.exe takes a string, never -name or -type: Linux's find was meant.
        if ($name -eq 'find' -and $cmd.CommandType -eq 'Application' -and $cmd.Source -like "$env:SystemRoot*" -and $words.Count -gt 1) { $ok = $true }
        # A real program (Git's ls.exe, say), a profile's function or a changed alias: it works as typed.
        if (-not $ok) { return $false }
    }
    $args1 = @(); if ($words.Count -gt 1) { $args1 = @($words[1..($words.Count - 1)]) }
    return & $rule $args1 $piped
}

# The line to run instead of $line, or $null to run it as typed.
function global:__ScoutConvert([string]$line) {
    $trim = $line.Trim()
    if (-not $trim -or $trim.Contains("`n")) { return $null }
    # Windows PowerShell's curl is Invoke-WebRequest, which takes none of curl's flags. Windows ships the real one.
    if (-not $__scoutIsPs7 -and $trim -match '^curl\s' -and (Get-Command curl -ErrorAction SilentlyContinue).CommandType -eq 'Alias' -and (Test-Path "$env:SystemRoot\System32\curl.exe")) {
        return 'curl.exe' + $trim.Substring(4)
    }
    # Quick way out for ordinary PowerShell: no command this knows, and nothing Windows PowerShell can't chain.
    $need = (-not $__scoutIsPs7) -and $trim -match '&&|\|\|'
    foreach ($m in [regex]::Matches($trim, '(?:^|[|;&])\s*([^\s|;&]+)')) { if ($__scoutRules.ContainsKey($m.Groups[1].Value)) { $need = $true } }
    if (-not $need) { return $null }

    $tokens = __ScoutTokens $trim
    if (-not $tokens -or $tokens.Count -eq 0) { return $null }

    # Pipelines joined by && || ;, each a list of segments joined by |.
    $pipes = New-Object System.Collections.ArrayList
    $joins = New-Object System.Collections.ArrayList
    $segs = New-Object System.Collections.ArrayList
    $cur = New-Object System.Collections.ArrayList
    foreach ($t in $tokens) {
        if ($t.op) {
            if ($cur.Count -eq 0) { return $null }
            [void]$segs.Add(@($cur)); $cur = New-Object System.Collections.ArrayList
            if ($t.op -ne '|') { [void]$pipes.Add($segs.ToArray()); $segs = New-Object System.Collections.ArrayList; [void]$joins.Add($t.op) }
        } else { [void]$cur.Add($t) }
    }
    if ($cur.Count -eq 0) { return $null }
    [void]$segs.Add(@($cur)); [void]$pipes.Add($segs.ToArray())

    $changed = $false
    $texts = foreach ($pl in $pipes) {
        $parts = for ($k = 0; $k -lt $pl.Count; $k++) {
            $words = @($pl[$k])
            $r = __ScoutSegment $words ($k -gt 0)
            if ($null -eq $r) { return $null }
            if ($r -eq $false) { ($words | ForEach-Object { if ($_.q -or $_.t -match '\s') { __ScoutQ $_.t } else { $_.t } }) -join ' ' }
            else { $changed = $true; $r }
        }
        $parts -join ' | '
    }
    $texts = @($texts)

    $ps7 = $__scoutIsPs7
    if (-not $ps7 -and ($joins -contains '&&' -or $joins -contains '||')) { $changed = $true }
    if (-not $changed) { return $null }

    # Windows PowerShell has no && or ||. Split on ; first, then each chain must be all && or all || to be certain.
    $out = New-Object System.Collections.ArrayList
    $chain = @($texts[0]); $ops = @()
    $flush = {
        if ($ps7 -or $ops.Count -eq 0) {
            $s = $chain[0]; for ($q = 0; $q -lt $ops.Count; $q++) { $s += " $($ops[$q]) " + $chain[$q + 1] }; return $s
        }
        if (@($ops | Select-Object -Unique).Count -ne 1) { return $null }
        $test = '$?'; if ($ops[0] -eq '||') { $test = '-not $?' }
        $s = $chain[$chain.Count - 1]
        for ($q = $chain.Count - 2; $q -ge 0; $q--) { $s = $chain[$q] + "; if ($test) { " + $s + ' }' }
        return $s
    }
    for ($j = 0; $j -lt $joins.Count; $j++) {
        if ($joins[$j] -eq ';') {
            $x = & $flush; if ($null -eq $x) { return $null }; [void]$out.Add($x)
            $chain = @($texts[$j + 1]); $ops = @()
        } else { $ops += $joins[$j]; $chain += $texts[$j + 1] }
    }
    $x = & $flush; if ($null -eq $x) { return $null }; [void]$out.Add($x)
    return ($out -join '; ')
}

function global:__ScoutBindEnter {
    # The panel draws its own suggestions; PSReadLine 2.1+ would draw a second set over them.
    try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}
    $enter = Get-PSReadLineKeyHandler -Bound -ErrorAction SilentlyContinue | Where-Object { $_.Key -eq 'Enter' } | Select-Object -First 1
    # Left alone when a profile has its own Enter.
    if ($enter -and $enter.Function -ne 'AcceptLine') { return }
    Set-PSReadLineKeyHandler -Key Enter -BriefDescription ScoutAcceptLine -Description 'Runs the line, as its PowerShell equivalent when it is a Linux command' -ScriptBlock {
        $line = $null; $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $new = $null
        try { $new = __ScoutConvert $line } catch { $new = $null }
        if ($new -and $new -ne $line) { [Microsoft.PowerShell.PSConsoleReadLine]::Replace(0, $line.Length, $new) }
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}

# Bound once the first prompt is up and waiting: reading the current key handlers takes over 100 ms, which the prompt
# shouldn't wait for.
$null = Register-EngineEvent -SourceIdentifier PowerShell.OnIdle -MaxTriggerCount 1 -Action { if (Get-Module PSReadLine) { __ScoutBindEnter } }

# Where the prompt ends, for the panel's suggestions: an invisible OSC 133;B after it (the shell-integration mark).
# Wraps whatever prompt the profile set up.
$global:__scoutPrompt = $function:prompt
# Before it, the folder the shell is in (OSC 7), so path suggestions come from where you are.
function global:prompt {
    $p = & $global:__scoutPrompt
    $e = [char]27; $b = [char]7
    $here = $ExecutionContext.SessionState.Path.CurrentLocation
    $dir = ""
    if ($here.Provider.Name -eq "FileSystem") { $dir = "$e]7;file:///" + ($here.ProviderPath -replace "\\", "/") + $b }
    "$dir$p$e]133;B$b"
}
