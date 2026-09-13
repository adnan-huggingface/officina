# officina-word-service.ps1 — a small HTTP front for Microsoft Word on this laptop.
#
# Runs in the logged-in desktop session (Word automation needs one), listens on
# one TCP port for the local network, and does five things for a caller that
# presents the shared token: keeps a work directory of files, runs the repo's
# probe scripts against them, opens a document in Word and reports what Word
# made of it, lists the fonts this machine has, and pulls the repo. Nothing
# here is reachable without the token except /health, which says only that the
# service is up and which Word it has.
#
# Start:  powershell -NoProfile -ExecutionPolicy Bypass -File C:\officina-word\service.ps1
#         (a copy of this file; the token is C:\officina-word\token.txt, and the
#         caller sets OFFICINA_WORD_SERVICE and OFFICINA_WORD_TOKEN — see
#         crates/wp-compare/src/reference.rs and README.md beside this)
# Log:    C:\officina-word\logs\service.log
param(
  [int]$Port = 8765,
  [string]$Root = "C:\officina-word"
)
$ErrorActionPreference = "Stop"

$Token = (Get-Content -LiteralPath (Join-Path $Root "token.txt") -Raw).Trim()
$Work  = Join-Path $Root "work"
$Repo  = Join-Path $Root "repo"
$Logs  = Join-Path $Root "logs"
foreach ($d in @($Work, (Join-Path $Work "scripts"), $Logs)) {
  New-Item -ItemType Directory -Force -Path $d | Out-Null
}
$Started = Get-Date

function Log([string]$msg) {
  $line = "{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $msg
  Add-Content -LiteralPath (Join-Path $Logs "service.log") -Value $line
}

# Word's identity, read once at start: every /health would otherwise cost a
# Word launch.
function WordInfo {
  try {
    $w = New-Object -ComObject Word.Application
    try {
      $w.Visible = $false
      return @{ ok = $true; version = "$($w.Version)"; build = "$($w.Build)" }
    } finally {
      $w.Quit()
      [void][Runtime.InteropServices.Marshal]::ReleaseComObject($w)
    }
  } catch {
    return @{ ok = $false; error = "$_" }
  }
}
$Word = WordInfo

function RepoCommit {
  try {
    if (Test-Path -LiteralPath (Join-Path $Repo ".git")) {
      return (& git -C $Repo rev-parse --short HEAD 2>$null | Out-String).Trim()
    }
    $stamp = Join-Path $Repo "SOURCE.txt"
    if (Test-Path -LiteralPath $stamp) { return (Get-Content -LiteralPath $stamp -Raw).Trim() }
  } catch {}
  return $null
}

# ---- responses ------------------------------------------------------------

function SendBytes($ctx, [int]$code, [byte[]]$bytes, [string]$type) {
  $r = $ctx.Response
  $r.StatusCode = $code
  $r.ContentType = $type
  $r.ContentLength64 = $bytes.Length
  if ($bytes.Length -gt 0) { $r.OutputStream.Write($bytes, 0, $bytes.Length) }
  $r.OutputStream.Close()
}
function SendJson($ctx, [int]$code, $obj) {
  $json = $obj | ConvertTo-Json -Depth 8
  SendBytes $ctx $code ([Text.Encoding]::UTF8.GetBytes($json)) "application/json; charset=utf-8"
}
function ReadBody($ctx) {
  $ms = New-Object IO.MemoryStream
  $ctx.Request.InputStream.CopyTo($ms)
  return $ms.ToArray()
}
function ReadJsonBody($ctx) {
  $bytes = ReadBody $ctx
  if ($bytes.Length -eq 0) { return @{} }
  $obj = [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
  $h = @{}
  foreach ($p in $obj.PSObject.Properties) { $h[$p.Name] = $p.Value }
  return $h
}

# ---- paths ----------------------------------------------------------------

# A work file is named "name.ext" or "dir/name.ext": one level, no tricks.
function WorkPath([string]$name) {
  if ([string]::IsNullOrWhiteSpace($name)) { throw "missing file name" }
  $parts = $name -split '/'
  if ($parts.Count -gt 2) { throw "at most one directory level: '$name'" }
  foreach ($p in $parts) {
    if ($p -eq '' -or $p -eq '.' -or $p -eq '..' -or $p -match '[\\:*?"<>|]') {
      throw "bad file name '$name'"
    }
  }
  return Join-Path $Work ($parts -join '\')
}

# A script is "repo/tools/probe/x.ps1", "repo/corpus/x.ps1" or
# "work/scripts/x.ps1" (the last is where the caller uploads its own).
function ScriptPath([string]$script) {
  if ([string]::IsNullOrWhiteSpace($script)) { throw "missing script" }
  if ($script -match '\.\.') { throw "no '..' in a script path" }
  $full = [IO.Path]::GetFullPath((Join-Path $Root ($script -replace '/', '\')))
  $allowed = @((Join-Path $Repo "tools\probe"), (Join-Path $Repo "corpus"), (Join-Path $Work "scripts"))
  $ok = $false
  foreach ($a in $allowed) {
    if ($full.StartsWith($a + '\', [StringComparison]::OrdinalIgnoreCase)) { $ok = $true }
  }
  if (-not $ok) { throw "a script must live under repo/tools/probe, repo/corpus or work/scripts: '$script'" }
  if (-not (Test-Path -LiteralPath $full)) { throw "no such script: '$script'" }
  if ([IO.Path]::GetExtension($full) -notin @('.ps1', '.py')) { throw "only .ps1 and .py scripts run: '$script'" }
  return $full
}

# "{work}/a.docx" and "{repo}/corpus/x.docx" in an argument become real paths.
function Expand([string]$v) {
  if ($v.StartsWith('{work}')) { return ($Work + ($v.Substring(6) -replace '/', '\')) }
  if ($v.StartsWith('{repo}')) { return ($Repo + ($v.Substring(6) -replace '/', '\')) }
  return $v
}

# ---- handlers -------------------------------------------------------------

function Health($ctx) {
  SendJson $ctx 200 @{
    ok = $true
    service = "officina-word"
    host = $env:COMPUTERNAME
    started = $Started.ToString("s")
    word = $Word
    repo_commit = (RepoCommit)
  }
}

function Fonts($ctx) {
  $keys = @(
    @{ scope = "machine"; key = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts"; dir = "$env:WINDIR\Fonts" },
    @{ scope = "user";    key = "HKCU:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts"; dir = "$env:LOCALAPPDATA\Microsoft\Windows\Fonts" }
  )
  $fonts = @()
  foreach ($k in $keys) {
    if (-not (Test-Path $k.key)) { continue }
    $item = Get-ItemProperty -Path $k.key
    foreach ($p in $item.PSObject.Properties) {
      if ($p.Name -like 'PS*') { continue }
      $file = "$($p.Value)"
      if (-not [IO.Path]::IsPathRooted($file)) { $file = Join-Path $k.dir $file }
      $fonts += @{ scope = $k.scope; name = $p.Name; file = $file }
    }
  }
  $cloud = @()
  $cloudDir = Join-Path $env:LOCALAPPDATA "Microsoft\FontCache\4\CloudFonts"
  if (Test-Path -LiteralPath $cloudDir) {
    foreach ($d in Get-ChildItem -LiteralPath $cloudDir -Directory) {
      $cloud += @{ family = $d.Name; files = @(Get-ChildItem -LiteralPath $d.FullName -File | ForEach-Object { $_.Name }) }
    }
  }
  SendJson $ctx 200 @{ fonts = $fonts; cloud = $cloud; count = $fonts.Count }
}

function ListFiles($ctx) {
  $list = @()
  foreach ($f in Get-ChildItem -LiteralPath $Work -File -Recurse) {
    $rel = $f.FullName.Substring($Work.Length + 1) -replace '\\', '/'
    $list += @{ name = $rel; size = $f.Length; modified = $f.LastWriteTime.ToString("s") }
  }
  SendJson $ctx 200 @{ files = $list }
}

function PutFile($ctx, [string]$name) {
  $path = WorkPath $name
  $dir = Split-Path -Parent $path
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $bytes = ReadBody $ctx
  [IO.File]::WriteAllBytes($path, $bytes)
  SendJson $ctx 200 @{ name = $name; size = $bytes.Length }
}

function GetFile($ctx, [string]$name) {
  $path = WorkPath $name
  if (-not (Test-Path -LiteralPath $path)) { SendJson $ctx 404 @{ error = "no such work file: $name" }; return }
  SendBytes $ctx 200 ([IO.File]::ReadAllBytes($path)) "application/octet-stream"
}

function DeleteFile($ctx, [string]$name) {
  $path = WorkPath $name
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
  SendJson $ctx 200 @{ name = $name; deleted = $true }
}

# POST /run  {"script": "repo/tools/probe/topdf.ps1",
#             "args": {"Path": "{work}/a.docx", "Out": "{work}/a.pdf"},
#             "argv": ["positional", "..."],  "timeout": 300}
# Named args become -Name Value; a boolean true becomes a bare -Switch.
# A .py script runs under python; its args are positional (argv).
function RunScript($ctx) {
  $req = ReadJsonBody $ctx
  $script = ScriptPath ([string]$req['script'])
  $timeout = 300
  if ($req['timeout']) { $timeout = [int]$req['timeout'] }
  $argv = New-Object System.Collections.Generic.List[string]
  $exe = 'powershell.exe'
  if ($script.EndsWith('.py')) {
    $exe = 'python.exe'
    $argv.Add("`"$script`"")
  } else {
    foreach ($a in @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$script`"")) { $argv.Add($a) }
  }
  if ($req['args']) {
    foreach ($p in $req['args'].PSObject.Properties) {
      if ($p.Value -is [bool]) { if ($p.Value) { $argv.Add("-$($p.Name)") }; continue }
      $argv.Add("-$($p.Name)")
      $argv.Add("`"$(Expand ([string]$p.Value))`"")
    }
  }
  if ($req['argv']) {
    foreach ($v in @($req['argv'])) { $argv.Add("`"$(Expand ([string]$v))`"") }
  }
  $t0 = Get-Date
  $stamp = $t0.ToString("yyyyMMdd-HHmmss")
  $outFile = Join-Path $Logs "run-$stamp-stdout.txt"
  $errFile = Join-Path $Logs "run-$stamp-stderr.txt"
  Log "run $exe $($argv -join ' ')"
  $proc = Start-Process -FilePath $exe -ArgumentList $argv.ToArray() -WorkingDirectory $Work `
            -WindowStyle Hidden -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
  # PS 5.1: without the handle cached now, ExitCode reads as null after exit.
  $null = $proc.Handle
  $finished = $proc.WaitForExit($timeout * 1000)
  $timedOut = -not $finished
  if ($timedOut) {
    Log "run timed out after ${timeout}s, killing pid $($proc.Id)"
    # PS 5.1 turns a native command's stderr into a terminating error under "Stop".
    & { $ErrorActionPreference = "Continue"; & taskkill.exe /T /F /PID $proc.Id 2>$null | Out-Null }
    Start-Sleep -Milliseconds 500
  }
  # A Word the script started and left behind would block the next request.
  $orphans = @(Get-Process WINWORD -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -gt $t0 })
  foreach ($o in $orphans) { try { $o.Kill() } catch {} }
  $code = $null
  try { $code = $proc.ExitCode } catch {}
  $stdout = ""; $stderr = ""
  try { $stdout = [IO.File]::ReadAllText($outFile) } catch {}
  try { $stderr = [IO.File]::ReadAllText($errFile) } catch {}
  SendJson $ctx 200 @{
    script = $req['script']
    exit_code = $code
    timed_out = $timedOut
    seconds = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
    stdout = $stdout
    stderr = $stderr
    orphaned_word_killed = $orphans.Count
  }
}

# POST /open?name=a.docx — open in Word, read-only, macros off, no dialogs;
# report what Word made of it.
function OpenReport($ctx) {
  $name = $ctx.Request.QueryString['name']
  $path = WorkPath $name
  if (-not (Test-Path -LiteralPath $path)) { SendJson $ctx 404 @{ error = "no such work file: $name" }; return }
  $t0 = Get-Date
  $result = @{ file = $name }
  $w = New-Object -ComObject Word.Application
  try {
    $w.Visible = $false
    $w.DisplayAlerts = 0            # wdAlertsNone
    $w.AutomationSecurity = 3       # msoAutomationSecurityForceDisable
    $doc = $null
    try {
      $doc = $w.Documents.Open($path, $false, $true)   # ConfirmConversions no, ReadOnly yes
      $result.opened = $true
      $result.pages = $doc.ComputeStatistics(2)         # wdStatisticPages
      $result.words = $doc.ComputeStatistics(0)         # wdStatisticWords
      $result.paragraphs = $doc.Paragraphs.Count
      $result.sections = $doc.Sections.Count
      $result.compatibility_mode = $doc.CompatibilityMode
      $result.save_format = $doc.SaveFormat
      $normal = $doc.Styles.Item(-1)                    # wdStyleNormal
      $result.normal_font = "$($normal.Font.Name)"
      $result.normal_size = $normal.Font.Size
      $result.normal_space_after = $normal.ParagraphFormat.SpaceAfter
      $result.normal_line_spacing = $normal.ParagraphFormat.LineSpacing
      $result.normal_line_spacing_rule = $normal.ParagraphFormat.LineSpacingRule
      $ps = $doc.PageSetup
      $result.page = @{ width = $ps.PageWidth; height = $ps.PageHeight; top = $ps.TopMargin; bottom = $ps.BottomMargin; left = $ps.LeftMargin; right = $ps.RightMargin }
    } catch {
      $result.opened = $false
      $result.error = "$_"
    } finally {
      if ($doc) { $doc.Close(0) }                        # wdDoNotSaveChanges
    }
  } finally {
    $w.Quit()
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($w)
  }
  $result.seconds = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
  SendJson $ctx 200 $result
}

function Pull($ctx) {
  if (-not (Test-Path -LiteralPath (Join-Path $Repo ".git"))) {
    SendJson $ctx 200 @{ pulled = $false; reason = "repo is not a git checkout"; commit = (RepoCommit) }; return
  }
  # git reports progress on stderr; under "Stop" PS 5.1 would throw on the first line.
  $out = & { $ErrorActionPreference = "Continue"; (& git -C $Repo pull --ff-only 2>&1 | ForEach-Object { "$_" } | Out-String) }
  $pulled = ($LASTEXITCODE -eq 0)
  SendJson $ctx 200 @{ pulled = $pulled; output = $out; commit = (RepoCommit) }
}

function Tail($ctx) {
  $n = 50
  if ($ctx.Request.QueryString['lines']) { $n = [int]$ctx.Request.QueryString['lines'] }
  $lines = @()
  $log = Join-Path $Logs "service.log"
  # "$_" drops the PSPath/PSProvider notes Get-Content adds: PS 5.1's ConvertTo-Json
  # would otherwise serialize each line as an object and walk the provider graph.
  if (Test-Path -LiteralPath $log) { $lines = @(Get-Content -LiteralPath $log -Tail $n | ForEach-Object { "$_" }) }
  SendJson $ctx 200 @{ lines = $lines }
}

# ---- loop -----------------------------------------------------------------

$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://+:$Port/")
$listener.Start()
Log "listening on port $Port, root $Root, word $($Word | ConvertTo-Json -Compress)"

while ($listener.IsListening) {
  $ctx = $listener.GetContext()
  $req = $ctx.Request
  $method = $req.HttpMethod
  $path = [Uri]::UnescapeDataString($req.Url.AbsolutePath).TrimEnd('/')
  if ($path -eq '') { $path = '/' }
  try {
    if ($path -eq '/health' -or $path -eq '/') { Health $ctx; continue }
    if ($req.Headers['X-Officina-Token'] -ne $Token) {
      SendJson $ctx 401 @{ error = "bad or missing X-Officina-Token" }
      Log "401 $method $path from $($req.RemoteEndPoint)"
      continue
    }
    Log "$method $path from $($req.RemoteEndPoint)"
    switch -Regex ("$method $path") {
      '^GET /fonts$'          { Fonts $ctx; break }
      '^GET /files$'          { ListFiles $ctx; break }
      '^PUT /files/(.+)$'     { PutFile $ctx $Matches[1]; break }
      '^GET /files/(.+)$'     { GetFile $ctx $Matches[1]; break }
      '^DELETE /files/(.+)$'  { DeleteFile $ctx $Matches[1]; break }
      '^POST /run$'           { RunScript $ctx; break }
      '^POST /open$'          { OpenReport $ctx; break }
      '^POST /pull$'          { Pull $ctx; break }
      '^GET /log$'            { Tail $ctx; break }
      default                 { SendJson $ctx 404 @{ error = "no route: $method $path" } }
    }
  } catch {
    Log "error $method $path : $_"
    try { SendJson $ctx 500 @{ error = "$_" } } catch {}
  }
}
