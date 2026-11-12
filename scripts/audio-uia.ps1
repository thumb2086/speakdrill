#%speakdrill-audio-uia% -- SpeakDrill per-app audio endpoint setter
# Drives ONLY "Settings > System > Sound > Volume mixer" via UI Automation.
# No SendKeys, no screen coordinates: every write is an Expand/Select/Invoke semantic action.
# Contract: last stdout line is SPEAKDRILL_RESULT=<json>; log lines go to stderr.
#
# Row model on Windows 11 (verified 26H2):
#   - per app there is a chevron Button (ControlType.Button, Name == app caption)
#   - its raw child is Group "SystemSettings_Audio_Mixer_App<PID>_EntityItem"
#   - when expanded, two ComboBoxes "SystemSettings_Audio_Mixer_App<PID>_ComboBox"
#     appear as siblings, tree order = output device first, input device second.
# The PID token is what ties a row to its app; we never assume tree order across apps.
param(
  [Parameter(Mandatory)][string]$AppName,
  [ValidateSet('input','output','both')][string]$Mode = 'input',
  [Parameter(Mandatory)][string]$EndpointName,
  [switch]$DryRun,
  [switch]$AutoOpenSettings,
  [switch]$NoRestore,
  [string]$Pids = '',
  [int]$TimeoutSec = 25,
  [int]$PollMs = 250
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$CT = [System.Windows.Automation.ControlType]
$AE = [System.Windows.Automation.AutomationElement]
$TP = [System.Windows.Automation.TreeScope]

$AppName = $AppName.Trim()
$EndpointName = $EndpointName.Trim()
$pidSet = @()
if ($Pids) { $pidSet = @($Pids -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '^\d+$' }) }
$Deadline = (Get-Date).AddSeconds($TimeoutSec)

function Log([string]$m) { [Console]::Error.WriteLine((Get-Date -Format o) + ' ' + $m) }
function Result([int]$code, [hashtable]$obj) {
  $obj['ok'] = ($code -eq 0)
  $obj['code'] = $code
  Write-Output ('SPEAKDRILL_RESULT=' + ($obj | ConvertTo-Json -Compress -Depth 6))
  exit $code
}
function Cond([System.Windows.Automation.AutomationProperty]$prop, $value) {
  New-Object System.Windows.Automation.PropertyCondition($prop, $value)
}
function And([object[]]$conds) {
  New-Object System.Windows.Automation.AndCondition($conds)
}
function Find-Mixer {
  $root = $AE::RootElement
  $cond = Cond $AE::AutomationIdProperty 'SystemSettings_Audio_Mixer_AppList_ItemsControl'
  return $root.FindFirst($TP::Descendants, $cond)
}

# ---- locate the mixer list (open Settings ourselves if allowed) -------
$mixer = Find-Mixer
if (-not $mixer -and $AutoOpenSettings) {
  Log 'mixer not present; launching ms-settings:apps-volume'
  Start-Process 'ms-settings:apps-volume'
  while ((Get-Date) -lt $Deadline -and -not $mixer) {
    Start-Sleep -Milliseconds $PollMs
    $mixer = Find-Mixer
  }
}
if (-not $mixer) { Result 12 @{ stage='mixer'; err='volume-mixer list control not found (Settings not on the mixer page)' } }

# ---- resolve exactly one app row ---------------------------------------
# WinUI answers SOME conditions lazily and others never (ControlType == Group
# matches nothing in Descendants while AutomationId == X does). So we anchor on
# the chevron Button (ControlType.Button + Name == caption; that works), take
# the row's PID token from the entity-item Group's AutomationId, and require
# that token to belong to one of OUR processes.
# Caption candidates (in order): each target process's MainWindowTitle, then
# its exe FileDescription/ProductName, then -AppName.
# returns @{ btn; pid; combos } | $null (not listed) | @{ error=11 | 15 }
function Get-AppPidTokenFromButton($b) {
  $kids = $b.FindAll($TP::Children, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($k in $kids) {
    if ($k.Current.AutomationId -match '^SystemSettings_Audio_Mixer_App(\d+)_EntityItem$') { return $Matches[1] }
  }
  $desc = $b.FindAll($TP::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($k in $desc) {
    if ($k.Current.AutomationId -match '^SystemSettings_Audio_Mixer_App(\d+)_EntityItem$') { return $Matches[1] }
  }
  return $null
}
function Get-ProcessCaption([int]$procId) {
  $out = @()
  try {
    $p = Get-Process -Id $procId -ErrorAction Stop
    if ($p.MainWindowTitle -and $p.MainWindowTitle.Trim().Length -ge 2) { $out += $p.MainWindowTitle.Trim() }
    if ($p.Path) {
      try {
        $v = (Get-Item $p.Path -ErrorAction SilentlyContinue).VersionInfo
        foreach ($f in @($v.FileDescription, $v.ProductName)) {
          if ($f -and $f.Trim()) { $out += $f.Trim() }
        }
      } catch {}
    }
  } catch {}
  return $out
}
function Find-AppRow {
  $root = $AE::RootElement
  # ---- build caption candidates -----------------------------------------
  $cands = New-Object System.Collections.ArrayList
  foreach ($q in $pidSet) {
    foreach ($c in (Get-ProcessCaption ([int]$q))) { [void]$cands.Add($c) }
  }
  if ($AppName) { [void]$cands.Add($AppName) }

  # ---- find rows whose PID token matches ours (or any caption hit) -----
  $seen = @{}
  foreach ($cap in $cands) {
    if (-not $cap) { continue }
    $btnCond = And @(
      (Cond $AE::NameProperty $cap),
      (Cond $AE::ControlTypeProperty $CT::Button)
    )
    $btns = $root.FindAll($TP::Descendants, $btnCond)
    foreach ($b in $btns) {
      $tok = Get-AppPidTokenFromButton $b
      if (-not $tok) { continue }
      if ($pidSet.Count -gt 0 -and -not ($pidSet -contains $tok)) { continue }
      if (-not $seen.ContainsKey($tok)) { $seen[$tok] = $b }
    }
    if ($seen.Count -eq 1) { break }
  }
  if ($seen.Count -gt 1) { return @{ error=11; count=$seen.Count } }
  if ($seen.Count -eq 0) { return $null }
  $eid = @($seen.Keys)[0]
  $btn = $seen[$eid]
  $appPid = $eid
  $combos = $root.FindAll($TP::Descendants,
    (Cond $AE::AutomationIdProperty "SystemSettings_Audio_Mixer_App${appPid}_ComboBox"))
  # order: BoundingRectangle.Top when known (output above input), else tree order
  $combos = @($combos | Sort-Object { try { $_.Current.BoundingRectangle.Top } catch { [double]::MaxValue } })
  return @{ btn=$btn; pid=$appPid; combos=$combos; expanded=((Get-ExpandState $btn) -eq 'Expanded') }
}

function Get-ExpandState($el) {
  $ec = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$ec)) {
    return $ec.Current.ExpandCollapseState.ToString()
  }
  return 'Unknown'
}

$rowCtx = $null
do {
  $fr = Find-AppRow
  if ($fr -is [hashtable] -and $fr.error) {
    if ($fr.error -eq 11) { Result 11 @{ stage='row'; err='multiple rows match the caption'; count=$fr.count } }
    Result 15 @{ stage='row'; err=$fr.err }
  }
  $rowCtx = $fr
  Log "row poll: ctx=$(if ($fr) { 'btn set combos=' + $fr.combos.Count } else { 'null' })"
  if (-not $rowCtx) { Start-Sleep -Milliseconds $PollMs }
} while ((-not $rowCtx) -and ((Get-Date) -lt $Deadline))
if (-not $rowCtx) { Result 10 @{ stage='row'; err='app has no audio session / not listed in the mixer'; app=$AppName } }

# ---- expand only if needed (remember if we did it) --------------------
function ComboCount($ctx) {
  if (-not $ctx) { return 0 }
  return $ctx.combos.Count
}
$expandedByUs = $false
$initialState = Get-ExpandState $rowCtx.btn
if ($initialState -eq 'Expanded' -and ((ComboCount $rowCtx) -lt 2)) {
  # WinUI claims the row is expanded but the subtree is not realized yet; poll for it
  $reDeadline0 = (Get-Date).AddSeconds(8)
  do {
    Start-Sleep -Milliseconds $PollMs
    $tmp = Find-AppRow
    Log "expand-poll: ctx=$(if ($tmp) { 'combos=' + $(if ($tmp.combos) { $tmp.combos.Count } else { 'ERR ' + ($tmp.err) }) } else { 'null' })"
    if ($tmp -and $tmp.ContainsKey('btn')) { $rowCtx = $tmp }
  } while (((ComboCount $rowCtx) -lt 2) -and ((Get-Date) -lt $reDeadline0))
}
if ((ComboCount $rowCtx) -lt 2) {
  if ($DryRun) {
    Result 16 @{ stage='dryrun'; app=$AppName; mode=$Mode; note='row is collapsed; expand it in Settings (or run without -DryRun) to read current values'; target=$EndpointName }
  }
  if ((Get-ExpandState $rowCtx.btn) -eq 'Expanded') {
    # already expanded according to WinUI: do NOT toggle it closed; the subtree is just not queryable right now
    Result 15 @{ stage='refresh'; err='row expanded but its controls are not queryable after polling' }
  }
  $ec = $null
  if (-not $rowCtx.btn.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$ec)) {
    Result 15 @{ stage='expand'; err='chevron button lacks ExpandCollapsePattern' }
  }
  $ec.Expand()
  $expandedByUs = $true
  $reDeadline = (Get-Date).AddSeconds(8)
  do {
    Start-Sleep -Milliseconds $PollMs
    $rowCtx = Find-AppRow
    $ok = ($rowCtx -and $rowCtx.ContainsKey('btn') -and $rowCtx.combos.Count -ge 2)
  } while ((-not $ok) -and ((Get-Date) -lt $reDeadline))
  if (-not $ok) { Result 15 @{ stage='refresh'; err='row vanished after expand' } }
}
if ($rowCtx.combos.Count -lt 2) { Result 15 @{ stage='combos'; err="expected >=2 combos after expand, got $($rowCtx.combos.Count)" } }
$outIdx = 0; $inIdx = 1   # BoundingRectangle.Top ascending = output first, input second

function ComboValue($c) {
  $sp = $null
  [void]$c.TryGetCurrentPattern([System.Windows.Automation.SelectionPattern]::Pattern, [ref]$sp)
  if ($sp) {
    $sel = $sp.Current.GetSelection()
    if ($sel.Count) { return $sel[0].Current.Name.Trim() }
  }
  return $null
}

function Fresh-Combo($idx) {
  # Windows rebuilds the row subtree asynchronously after Expand/Select — poll briefly
  $deadlineC = (Get-Date).AddSeconds(6)
  do {
    $ctx = Find-AppRow
    if ($ctx -and $ctx.ContainsKey('btn') -and $ctx.combos.Count -gt $idx) {
      return $ctx.combos[$idx]
    }
    Start-Sleep -Milliseconds $PollMs
  } while ((Get-Date) -lt $deadlineC)
  throw "combo[$idx] not findable (row missing or unrealized)"
}

function Set-OneCombo($idx, $target, [bool]$dry) {
  $c = Fresh-Combo $idx
  $current = ComboValue $c
  if ($dry) { return @{ current=$current; target=$target; changed=$false; already=($current -eq $target) } }
  if ($current -eq $target) { return @{ current=$current; target=$target; changed=$false; already=$true } }

  for ($attempt = 1; $attempt -le 2; $attempt++) {
    $c = Fresh-Combo $idx
    $ec = $null
    if (-not $c.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$ec)) {
      throw 'combo lacks ExpandCollapsePattern'
    }
    $ec.Expand()

    $itemCond = Cond $AE::ControlTypeProperty $CT::ListItem
    $items = @()
    $itemDeadline = (Get-Date).AddSeconds(3)
    do {
      Start-Sleep -Milliseconds 300
      $items = @($c.FindAll($TP::Descendants, $itemCond))
      if (-not $items.Count) {
        $root = $AE::RootElement
        $items = @($root.FindAll($TP::Descendants, $itemCond))
      }
    } while ((-not $items.Count) -and ((Get-Date) -lt $itemDeadline))
    $allNames = @($items | ForEach-Object { $_.Current.Name.Trim() })
    $exact = @($items | Where-Object { $_.Current.Name.Trim() -eq $target.Trim() })
    if ($exact.Count -eq 0) {
      # device captions are "Short (Vendor Suffix)"; accept a unique prefix match
      $prefix = $target.Trim() + ' ('
      $exact = @($items | Where-Object { $_.Current.Name.Trim().StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) })
    }
    if ($exact.Count -ne 1) {
      try { $ec.Collapse() } catch {}
      Result 13 @{ stage='item'; err='endpoint not found exactly once'; available=$allNames; target=$target }
    }

    $item = $exact[0]
    $selectedName = $item.Current.Name.Trim()
    $sp2 = $null
    if ($item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$sp2)) {
      $sp2.Select()
    } else {
      $iv2 = $null
      if (-not $item.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$iv2)) { throw 'item not selectable' }
      $iv2.Invoke()
    }
    Start-Sleep -Milliseconds 400
    try { $ec.Collapse() } catch {}

    $after = ComboValue (Fresh-Combo $idx)
    if ($after -eq $selectedName) {
      return @{ current=$current; target=$target; changed=$true; already=$false; after=$after; attempts=$attempt }
    }
    Log "post-verify mismatch (got '$after', expected '$selectedName'), retrying once"
  }
  Result 14 @{ stage='verify'; err='post-verify did not match target after retry'; target=$target }
}

$plan = @()
try {
  if ($Mode -in @('output','both')) {
    $plan += [ordered]@{ role='output'; combo=$outIdx; result=(Set-OneCombo $outIdx $EndpointName ([bool]$DryRun)) }
  }
  if ($Mode -in @('input','both')) {
    $plan += [ordered]@{ role='input'; combo=$inIdx; result=(Set-OneCombo $inIdx $EndpointName ([bool]$DryRun)) }
  }
} catch {
  Result 15 @{ stage='set'; err=$_.Exception.Message }
}

# restore the row's expanded state only if we expanded it ourselves
if ($expandedByUs -and -not $NoRestore) {
  try {
    $ctx = Find-AppRow
    if ($ctx -and $ctx.ContainsKey('btn')) {
      $ec = $null
      if ($ctx.btn.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$ec)) {
        $ec.Collapse()
      }
    }
  } catch { Log ("restore row failed (left expanded): " + $_) }
}

$ok = $true
foreach ($p in $plan) {
  $r = $p.result
  if ($r.changed -ne $true -and $r.already -ne $true) { $ok = $false }
}
$code = if ($ok) { 0 } else { 14 }
Result $code @{ stage='done'; app=$AppName; mode=$Mode; plan=$plan; initialState=$initialState }
