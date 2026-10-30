<#
Checklist § 26 (conversation context) on Windows, on a throwaway profile.

  pwsh -File scripts\checklist-26.ps1 setup          # profile, demo repository, the dev ys
  pwsh -File scripts\checklist-26.ps1 check [name]   # what can be read from outside the window
  pwsh -File scripts\checklist-26.ps1 clean          # remove everything setup made

PowerShell 7 (pwsh), as the justfile is: Windows PowerShell 5.1 reads a file without a BOM as
the ANSI code page, and the dashes and arrows here would not survive it.

macOS and Linux have scripts/checklist-26.sh, which this follows. The rows are in
docs/design/08-manual-checklist.md; on Windows they are the first two and the one with the other
five agents.

The window is still yours to click: this prepares what the rows need and then checks what they
leave behind — the conversations `ys workspace context` reads, whether Codex ran it before
answering, and which running agents were started with the note, whole. What only the window
shows (the line under the tabs, the mark, Show) is yours to look at.

Agents must find this build's ys, which has `workspace context`, before any installed one. On
Windows the app keeps the PATH it was started with and adds the registry's after it
(crates/core/src/env.rs), so the launch line puts target\debug first. Needs git.
#>
param(
    [Parameter(Position = 0)] [string] $Command = "help",
    [Parameter(Position = 1)] [string] $Name = ""
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot

# LOCALAPPDATA, not TEMP: TEMP can be spelt the short 8.3 way (C:\Users\RUNNER~1), and a path
# spelt two ways is a finding of its own, not one to stumble on here.
$root = if ($env:YS_C26_ROOT) { $env:YS_C26_ROOT } else { Join-Path $env:LOCALAPPDATA "Temp\ys-c26" }
$ysExe = Join-Path $repo "target\debug\ys.exe"

# ys against the throwaway profile, and never against an enclosing agent's workspace. A script
# runs in the caller's session, so what is cleared is put back.
function Invoke-Ys {
    $names = "YARDSORT_RUN_ID", "YARDSORT_WORKSPACE_ID", "YARDSORT_SESSION_RECORD_ID"
    $saved = @{}
    foreach ($n in $names) {
        $saved[$n] = [Environment]::GetEnvironmentVariable($n)
        [Environment]::SetEnvironmentVariable($n, $null)
    }
    try {
        & $ysExe --data-dir (Join-Path $root "data") @args
        if ($LASTEXITCODE -ne 0) { throw "ys $args failed" }
    } finally {
        foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $saved[$n]) }
    }
}

function Show-Steps {
    @"

Start the app on the throwaway profile, from this folder, in a new PowerShell window (these
settings last as long as it does):

  `$env:Path = "$repo\target\debug;`$env:Path"
  `$env:YARDSORT_DATA_DIR = "$root\data"
  `$env:YARDSORT_WORKTREE_ROOT = "$root\wt"
  `$env:YARDSORT_YS_DIR = "$root\ys-install"
  just dev

Then, in the window:

  1. Add project → $root\demo\notes-app. Memory… for it: "Share conversations between this
     project's agents" is ticked. Create a workspace in it (a worktree); call it tidy-calc.
  2. Start Claude from the tab bar and ask:
       In one sentence, what does calc.py do? Don't change anything.
     Start Codex from the tab bar: no question, no message, no turn. A line under the tabs says
     "Told about claude's conversation" for a minute; a ⇄ mark stays on the tab.
  3. Ask Codex:
       What was the other agent asked?
     It runs ``ys workspace context`` before answering, and answers from it. Show on the mark:
     the note, and the digest without Codex's own conversation.
  4. Start Cursor, OpenCode, Grok, OMP and pi in the workspace, and give each a message:
       In one sentence, what is in README.md? Don't change anything.
     Grok, OMP and pi start from the tab bar without the question, told as background context.
  5. Quit OMP and Resume it: its tab says "Told about …" again. Quit Grok and resume it: it does
     not. While both resumed tabs are open, run:

       pwsh -File scripts\checklist-26.ps1 check tidy-calc

  6. Show on a new agent's line, or the digest check prints: every conversation is there with
     its messages, and none of the tools they ran.

"@
}

function Invoke-Setup {
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { throw "needs git" }
    if (Test-Path $root) { Remove-Item -Recurse -Force $root }
    foreach ($dir in "data", "wt", "demo\notes-app") {
        New-Item -ItemType Directory -Force (Join-Path $root $dir) | Out-Null
    }

    Write-Host "Building ys…"
    Push-Location $repo
    try { cargo build -q -p yardsort-cli; if ($LASTEXITCODE -ne 0) { throw "cargo build failed" } }
    finally { Pop-Location }

    # The demo repository the questions are about. Small enough that every answer is one sentence.
    $demo = Join-Path $root "demo\notes-app"
    Push-Location $demo
    try {
        git init -q -b main
        # Written without a BOM, with LF endings, as the macOS and Linux run has them.
        $utf8 = New-Object System.Text.UTF8Encoding $false
        [IO.File]::WriteAllText("$demo\README.md", "# notes-app`n`nA tiny demo repository for trying Yardsort.`n", $utf8)
        [IO.File]::WriteAllText("$demo\calc.py", "def add(a, b):`n    return a + b`n", $utf8)
        [IO.File]::WriteAllText("$demo\notes.txt", "Remember to water the plants.`n", $utf8)
        git add -A
        git -c user.name=Demo -c user.email=demo@example.invalid commit -q -m "First commit"
    } finally { Pop-Location }

    # The digest only counts what was written after this, when it looks for Codex's run.
    Set-Content -Path (Join-Path $root "stamp") -Value (Get-Date -Format o)

    Write-Host "Set up under $root; this build's ys: $(& $ysExe --version)."
    Show-Steps
}

# The one worktree workspace in the profile, or the one named.
function Find-Workspace([string] $wanted) {
    $spaces = @(Invoke-Ys workspace list --json | Out-String | ConvertFrom-Json |
        Where-Object { $_.kind -eq "worktree" -and -not $_.archived })
    if ($wanted) {
        $spaces = @($spaces | Where-Object { $_.name -eq $wanted -or $_.id.StartsWith($wanted) })
    }
    if ($spaces.Count -ne 1) {
        $names = if ($spaces.Count) { ($spaces | ForEach-Object name) -join ", " } else { "none" }
        throw "Name the workspace: worktree workspaces here are $names."
    }
    $spaces[0]
}

function Write-Verdict([string] $verdict, [string] $text) {
    Write-Host ("{0,-5} {1}" -f $verdict, $text)
}

function Invoke-Check([string] $wanted) {
    if (-not (Test-Path (Join-Path $root "data\yardsort.db"))) {
        throw "No profile at $root\data yet: run setup, then start the app as it says."
    }
    $ws = Find-Workspace $wanted
    Write-Host "Workspace $($ws.name), at $($ws.path)`n"

    try { Invoke-Ys workspace context $ws.name | Set-Content -Encoding utf8 (Join-Path $root "context.md") }
    catch {
        Write-Verdict "FAIL" "ys workspace context refused; is sharing on in the project's Memory…?"
        exit 1
    }
    $digest = @((Invoke-Ys workspace context $ws.name --json | Out-String | ConvertFrom-Json).conversations)
    Write-Host "The digest is in $root\context.md — read it for tool output, which must not be there.`n"

    $said = @{}
    foreach ($c in $digest) {
        if (-not $said.ContainsKey($c.harness)) { $said[$c.harness] = @() }
        $said[$c.harness] += $c
    }
    $spoke = { param($h) [bool]($said[$h] | Where-Object { $_.turnCount -gt 0 }) }

    # Every running process started with the note, by the agent it belongs to.
    $marker = "Other agents have worked in this workspace:"
    $harnesses = "claude", "codex", "cursor-agent", "opencode", "grok", "omp", "pi"
    $told = @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($marker) } |
        ForEach-Object {
            $process = $_
            $line = $process.CommandLine
            $before = $line.Substring(0, $line.IndexOf($marker))
            $tokens = @($before -split '[\s"]+' | Where-Object { $_ })
            $names = @($tokens | ForEach-Object { [IO.Path]::GetFileNameWithoutExtension($_) })
            $program = ($names | Where-Object { $harnesses -contains $_ } | Select-Object -First 1)
            if (-not $program) { $program = $process.Name }
            [pscustomobject]@{
                Pid     = $process.ProcessId
                Program = $program
                Flag    = if ($tokens.Count) { $tokens[-1] } else { "" }
                # The whole note, or the command line lost the end of it on the way.
                Whole   = $line.Contains("refers to their work.")
            }
        })
    $toldAs = { param($h) @($told | Where-Object { $_.Program -eq $h }) }

    Write-Host "Row 1 · Claude, then Codex from the tab bar"
    Write-Verdict $(if (& $spoke "claude") { "PASS" } else { "FAIL" }) "a Claude conversation with a turn is in the digest"
    if ((& $toldAs "codex").Count) { Write-Verdict "PASS" "a running Codex was started with the note" }
    else { Write-Verdict "LOOK" "no running Codex carries the note; check while its first tab is open" }

    Write-Host "`nRow 2 · Codex reads the conversation before answering"
    $asked = @($said["codex"] | ForEach-Object { $_.turns } | Where-Object { $_.user -match "other agent" })
    Write-Verdict $(if ($asked.Count) { "PASS" } else { "FAIL" }) "Codex was asked what the other agent was asked"
    # Codex writes its session files under CODEX_HOME; the one that ran the command since setup.
    $codexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME ".codex" }
    $since = (Get-Item (Join-Path $root "stamp")).LastWriteTime
    $leaf = Split-Path -Leaf $ws.path
    $ran = $null
    if (Test-Path "$codexHome\sessions") {
        $ran = Get-ChildItem "$codexHome\sessions" -Recurse -Filter *.jsonl |
            Where-Object { $_.LastWriteTime -gt $since } |
            Where-Object { (Select-String -Path $_.FullName -SimpleMatch "workspace context" -Quiet) -and
                (Select-String -Path $_.FullName -SimpleMatch $leaf -Quiet) } |
            Select-Object -First 1
    }
    Write-Verdict $(if ($ran) { "PASS" } else { "FAIL" }) ("Codex ran ``ys workspace context`` in this workspace" + $(if ($ran) { " ($($ran.Name))" } else { "" }))
    if ($asked.Count -and $asked[0].reply) {
        $answer = ($asked[0].reply -split '\s+') -join ' '
        Write-Host "      its answer: $($answer.Substring(0, [Math]::Min(200, $answer.Length)))"
    }

    Write-Host "`nRow 7 · the other five agents"
    foreach ($h in "cursor", "opencode", "grok", "omp", "pi") {
        if (-not $said.ContainsKey($h)) { Write-Verdict "FAIL" "no $h conversation in the digest" }
        else { Write-Verdict $(if (& $spoke $h) { "PASS" } else { "FAIL" }) "$h's messages are in the digest" }
    }
    if ((& $toldAs "omp").Count) { Write-Verdict "PASS" "a running OMP carries the note" }
    else { Write-Verdict "LOOK" "a running OMP carries the note; check while the resumed tab is open" }
    Write-Verdict "LOOK" "running Grok processes with the note: $((& $toldAs "grok").Count) — after a resume there should be none"

    $unread = @($digest | Where-Object { $_.unreadable })
    if ($unread.Count) {
        Write-Host "`nNot read — fine for one opened and closed without a message:"
        foreach ($c in $unread) { Write-Host "      $($c.harness) $($c.id): $($c.unreadable)" }
    }

    Write-Host "`nRunning agents started with the note:"
    foreach ($t in $told) {
        Write-Host ("  {0,7}  {1} {2}<the note>{3}" -f $t.Pid, $t.Program, $t.Flag, $(if ($t.Whole) { "" } else { "  ← cut short" }))
    }
    if (-not $told.Count) { Write-Host "  none" }
    if ($told | Where-Object { -not $_.Whole }) { Write-Verdict "FAIL" "a note did not reach its agent whole" }

    Write-Host ""
    Show-Results
}

function Show-Results {
    $os = Get-CimInstance Win32_OperatingSystem
    $cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    $version = (& $ysExe --version) -replace '^ys\s+', ''
    @"
For 08 § Results, once every row passes in the window too:

### $($os.Caption) $($os.BuildNumber), $cpu, $version (dev) — $(Get-Date -Format yyyy-MM-dd)
§ 26: the first two rows and the row with the other five agents pass.
Found: …
"@
}

function Invoke-Clean {
    if (Test-Path $root) { Remove-Item -Recurse -Force $root }
    Write-Host "Removed $root. The agents' own conversations (~\.claude, ~\.codex, …) are kept."
}

switch ($Command) {
    "setup" { Invoke-Setup }
    "steps" { Show-Steps }
    "check" { Invoke-Check $Name }
    "clean" { Invoke-Clean }
    default {
        Write-Host "  pwsh -File scripts\checklist-26.ps1 setup | check [name] | clean"
        if ($Command -ne "help") { exit 1 }
    }
}
