<#
.SYNOPSIS
  Generates the fidelity corpus by driving Microsoft Word and Excel through COM.

.DESCRIPTION
  The corpus has to be real Office output — that is the whole point of it. Rather
  than collecting files of uncertain provenance, this drives the real applications
  and asks them to produce documents exercising the features most likely to be
  silently destroyed by a naive round trip.

  This script is self-contained. Copy just this one file to a machine with Word
  and Excel, run it, and send back the zip it produces. It needs no other part of
  the repository and writes nothing outside its output directory.

.PARAMETER OutDir
  Where to write docx/, xlsx/, the manifest, and the zip.
  Defaults to a `corpus-out` folder beside this script.

.PARAMETER NoZip
  Leave the loose files without packaging them.

.PARAMETER SkipPreflight
  Skip the up-front check. Only useful if it misfires on a working install.

.PARAMETER TimeoutSec
  How long one document may take before it is abandoned. Default 240.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\generate.ps1

.NOTES
  Every document is generated in a *child process*, one at a time, under a
  timeout. That is not defensive programming for its own sake — driving Word
  through COM from inside the running PowerShell session reliably hangs on this
  and other machines: `Documents.Add()` never returns, Word sits spinning at 100%
  CPU, and no error is ever raised. The identical calls in a child process work
  fine, which is what the preflight check demonstrated before this rewrite.

  The cost is one Office startup per document, a couple of minutes over the whole
  run. The benefit is that a single hung document loses that document instead of
  the entire corpus.

  Generated documents are clean by construction. They do not carry the accumulated
  oddities of a document edited across many Word versions, which is where the
  nastiest preservation bugs live. Supplement with real-world files where possible.
#>

[CmdletBinding()]
param(
    [string]$OutDir,
    [switch]$NoZip,
    [switch]$SkipPreflight,
    [int]$TimeoutSec = 240,
    # Wildcards accepted, e.g. -Only pivot*,number* to retry just those two.
    [string[]]$Only
)

$ErrorActionPreference = 'Stop'

# `$PSScriptRoot` is empty under some invocation paths, which turned into an
# unhelpful "cannot bind argument to parameter 'Path'" from Join-Path before the
# script had run a single line. Resolved here, with the current directory as the
# fallback, rather than in the param block where it cannot be checked.
if (-not $OutDir) {
    $base = $PSScriptRoot
    if (-not $base) { $base = Split-Path -Parent $MyInvocation.MyCommand.Path }
    if (-not $base) { $base = (Get-Location).Path }
    $OutDir = Join-Path $base 'corpus-out'
}

$docxDir = Join-Path $OutDir 'docx'
$xlsxDir = Join-Path $OutDir 'xlsx'
New-Item -ItemType Directory -Force -Path $docxDir | Out-Null
New-Item -ItemType Directory -Force -Path $xlsxDir | Out-Null

$script:Made    = @()
$script:Skipped = @()
$script:Versions = @{}

# Office instances already running belong to the user, not to us. Recorded so the
# cleanup below never kills a document they had open.
$script:PreexistingOffice = @(
    Get-Process WINWORD, EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }
)

function Stop-StrayOfficeProcesses {
    foreach ($p in (Get-Process WINWORD, EXCEL -ErrorAction SilentlyContinue)) {
        if ($script:PreexistingOffice -notcontains $p.Id) {
            try { Stop-Process -Id $p.Id -Force -ErrorAction Stop } catch { }
        }
    }
}

trap {
    Write-Host "`nfatal: $($_.Exception.Message)" -ForegroundColor Red
    Get-Job -ErrorAction SilentlyContinue | Remove-Job -Force -ErrorAction SilentlyContinue
    Stop-StrayOfficeProcesses
    break
}

# ------------------------------------------------------------- Generation ----

# Runs in the child process. Creates the app, hands it to the generator body,
# and shuts it down whatever happens.
$Runner = {
    param($ProgId, $Path, $BodyText, $Png)
    $ErrorActionPreference = 'Stop'
    $app = New-Object -ComObject $ProgId
    $app.Visible = $false
    try { $app.DisplayAlerts = 0 } catch { }
    try {
        $body = [scriptblock]::Create($BodyText)
        & $body $app $Path $Png
    } catch {
        # Reported as ordinary output rather than left in the job's error stream:
        # a terminating error inside a job is awkward to retrieve from the parent,
        # and "no file produced" with no reason is useless when a generator fails.
        "ERROR: $($_.Exception.Message)"
    } finally {
        try { $app.Quit() } catch { }
    }
}

function Invoke-Doc {
    <#
      Generates one document in its own process, under a timeout.

      Success is judged by whether the file exists afterwards, not by whether the
      child reported an error — some COM calls report failure for a step the
      document survived, and a file on disk is the thing we actually wanted.
    #>
    param(
        [string]$Name,
        [ValidateSet('Word', 'Excel')][string]$App,
        [string]$Dir,
        [scriptblock]$Body
    )

    if ($Only -and -not ($Only | Where-Object { $Name -like $_ })) { return }

    # Printed before the work, not after: a COM call that never returns produces
    # no error and no output, so without this a stall is indistinguishable from
    # slowness and there is nothing to report about which document caused it.
    Write-Host ("  ...     $Name") -ForegroundColor DarkGray -NoNewline

    $path = Join-Path $Dir $Name
    if (Test-Path $path) { Remove-Item $path -Force }

    $progId = if ($App -eq 'Word') { 'Word.Application' } else { 'Excel.Application' }

    $job = Start-Job -ScriptBlock $Runner `
        -ArgumentList $progId, $path, $Body.ToString(), $script:PngPath

    $finished = Wait-Job $job -Timeout $TimeoutSec
    $failure = $null

    if (-not $finished) {
        $failure = "timed out after ${TimeoutSec}s"
        Stop-Job $job -ErrorAction SilentlyContinue
    } else {
        $output = @(Receive-Job $job -ErrorAction SilentlyContinue 2>&1)
        $reported = $output | Where-Object { "$_" -like 'ERROR: *' } | Select-Object -First 1
        if ($reported) { $failure = ("$reported" -replace '^ERROR: ', '') }
    }
    Remove-Job $job -Force -ErrorAction SilentlyContinue
    Stop-StrayOfficeProcesses

    if (Test-Path $path) {
        $script:Made += $Name
        Write-Host "`r  ok      $Name                                        " -ForegroundColor Green
    } else {
        if (-not $failure) { $failure = 'no file produced' }
        $script:Skipped += "$Name  ($failure)"
        Write-Host "`r  skipped $Name  -> $failure" -ForegroundColor Yellow
    }
}

# ------------------------------------------------------------- Preflight ------

function Test-OfficeScriptable {
    param([string]$ProgId, [string]$Extension, [int]$SaveFormat, [int]$Timeout = 120)

    $probe = Join-Path ([IO.Path]::GetTempPath()) ("st19-probe-" + [IO.Path]::GetRandomFileName() + $Extension)

    $job = Start-Job -ScriptBlock {
        param($ProgId, $Path, $Fmt)
        $app = New-Object -ComObject $ProgId
        $app.Visible = $false
        try { $app.DisplayAlerts = 0 } catch { }
        try {
            if ($ProgId -like 'Word*') {
                $d = $app.Documents.Add()
                $d.Content.Text = 'probe'
                $d.SaveAs2($Path, $Fmt)
                $d.Close(0)
            } else {
                $b = $app.Workbooks.Add()
                $b.Worksheets.Item(1).Range('A1').Value2 = 'probe'
                $b.SaveAs($Path, $Fmt)
                $b.Close($false)
            }
        } finally {
            try { $app.Quit() } catch { }
        }
    } -ArgumentList $ProgId, $probe, $SaveFormat

    $finished = Wait-Job $job -Timeout $Timeout
    $reason = $null

    if (-not $finished) {
        Stop-Job $job -ErrorAction SilentlyContinue
        $reason = "no response after ${Timeout}s — Office is probably showing an activation dialog (unlicensed / 'read and print only mode'). Automation cannot dismiss it."
    } else {
        $out = Receive-Job $job -ErrorAction SilentlyContinue 2>&1
        if (-not (Test-Path $probe)) { $reason = "the probe produced no file. $out".Trim() }
    }

    Remove-Job $job -Force -ErrorAction SilentlyContinue
    if (Test-Path $probe) { Remove-Item $probe -Force -ErrorAction SilentlyContinue }
    Stop-StrayOfficeProcesses

    return [pscustomobject]@{ Ok = ($null -eq $reason); Reason = $reason }
}

$doWord = $true
$doExcel = $true

if (-not $SkipPreflight) {
    Write-Host "Preflight (one throwaway save per app)" -ForegroundColor Cyan

    $w = Test-OfficeScriptable -ProgId 'Word.Application' -Extension '.docx' -SaveFormat 12
    if ($w.Ok) {
        Write-Host "  ok      Word can create and save" -ForegroundColor Green
    } else {
        $doWord = $false
        Write-Host "  BLOCKED Word: $($w.Reason)" -ForegroundColor Red
    }

    $x = Test-OfficeScriptable -ProgId 'Excel.Application' -Extension '.xlsx' -SaveFormat 51
    if ($x.Ok) {
        Write-Host "  ok      Excel can create and save" -ForegroundColor Green
    } else {
        $doExcel = $false
        Write-Host "  BLOCKED Excel: $($x.Reason)" -ForegroundColor Red
    }

    if (-not $doWord -and -not $doExcel) {
        Write-Host "`nNeither application can save. Nothing to generate." -ForegroundColor Red
        exit 1
    }
}

# A small PNG for the image-wrapping cases, so the script has no external assets.
$script:PngPath = Join-Path $OutDir 'sample-image.png'
if (-not (Test-Path $script:PngPath)) {
    Add-Type -AssemblyName System.Drawing
    $bmp = New-Object System.Drawing.Bitmap 160, 120
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(70, 110, 190))
    $g.FillEllipse([System.Drawing.Brushes]::Gold, 30, 20, 100, 80)
    $g.Dispose()
    $bmp.Save($script:PngPath, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

# ---------------------------------------------------------------- Word --------
# Each body runs in a child process and receives ($word, $path, $png).

if ($doWord) {
    Write-Host "`nWord" -ForegroundColor Cyan

    Invoke-Doc 'minimal.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "A single paragraph. The baseline case."
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'styles-headings-toc.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        foreach ($h in @(@('Introduction', 1), @('Background', 2), @('Method', 2), @('Results', 1))) {
            $p = $d.Paragraphs.Add()
            $p.Range.Text = $h[0]
            $p.Range.Style = "Heading $($h[1])"
            $body = $d.Paragraphs.Add()
            $body.Range.Text = "Body text under $($h[0]). " * 4
            $body.Range.Style = 'Normal'
        }
        # A TOC field: fields are a classic thing to lose on round trip.
        $d.TablesOfContents.Add($d.Range(0, 0), $true, 1, 3) | Out-Null
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'tracked-changes.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "The original sentence stays here. This sentence will be deleted."
        $d.TrackRevisions = $true
        $p = $d.Paragraphs.Add()
        $p.Range.Text = "This paragraph was inserted with revision tracking on."
        # Delete a stretch so the file carries both an insertion and a deletion.
        $d.Range(35, 64).Delete() | Out-Null
        $d.TrackRevisions = $false
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'comments.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "A sentence that reviewers argued about at some length."
        $c = $d.Comments.Add($d.Range(2, 10), "Is this the right word?")
        # A reply, which is stored differently from a top-level comment.
        try { $c.Replies.Add($c.Range, "I think so.") | Out-Null } catch { }
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'nested-tables.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $outer = $d.Tables.Add($d.Range(0, 0), 3, 3)
        $outer.Borders.Enable = $true
        for ($r = 1; $r -le 3; $r++) {
            for ($c = 1; $c -le 3; $c++) { $outer.Cell($r, $c).Range.Text = "R$r C$c" }
        }
        # Table inside a cell: layout code that assumes a flat table breaks here.
        $inner = $d.Tables.Add($outer.Cell(2, 2).Range, 2, 2)
        $inner.Borders.Enable = $true
        $inner.Cell(1, 1).Range.Text = "inner"
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'table-spanning-pages.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $t = $d.Tables.Add($d.Range(0, 0), 60, 4)
        $t.Borders.Enable = $true
        $t.Rows.Item(1).HeadingFormat = $true   # repeats on each page
        for ($c = 1; $c -le 4; $c++) { $t.Cell(1, $c).Range.Text = "Header $c" }
        for ($r = 2; $r -le 60; $r++) {
            for ($c = 1; $c -le 4; $c++) { $t.Cell($r, $c).Range.Text = "r$r-c$c" }
        }
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'floating-image-wrap.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = ("Text that must flow around the floating image. " * 40)
        $shape = $d.Shapes.AddPicture($png, $false, $true, 100, 100, 160, 120)
        $shape.WrapFormat.Type = 2   # wdWrapSquare
        # An inline image too: inline and floating are different storage entirely.
        $d.InlineShapes.AddPicture($png, $false, $true, $d.Range(0, 0)) | Out-Null
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'footnotes-endnotes.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "A claim requiring a footnote. And another requiring an endnote."
        $d.Footnotes.Add($d.Range(30, 30), "", "The supporting footnote text.") | Out-Null
        $d.Endnotes.Add($d.Range(60, 60), "", "The supporting endnote text.") | Out-Null
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'sections-mixed-orientation.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "Portrait section."
        $d.Sections.Add() | Out-Null
        $d.Sections.Item(2).PageSetup.Orientation = 1   # wdOrientLandscape
        $d.Sections.Item(2).Range.InsertAfter("Landscape section with different page setup.")
        $d.Sections.Add() | Out-Null
        $d.Sections.Item(3).PageSetup.Orientation = 0
        $d.Sections.Item(3).PageSetup.LeftMargin = 144
        $d.Sections.Item(3).Range.InsertAfter("Third section, wider margins.")
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'headers-footers.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = ("Body text. " * 300)
        $ps = $d.Sections.Item(1).PageSetup
        $ps.DifferentFirstPageHeaderFooter = $true
        $ps.OddAndEvenPagesHeaderFooter = $true
        $s = $d.Sections.Item(1)
        $s.Headers.Item(2).Range.Text = "First page header"   # wdHeaderFooterFirstPage
        $s.Headers.Item(1).Range.Text = "Odd page header"     # wdHeaderFooterPrimary
        $s.Headers.Item(3).Range.Text = "Even page header"    # wdHeaderFooterEvenPages
        $s.Footers.Item(1).Range.Fields.Add($s.Footers.Item(1).Range, 33) | Out-Null  # wdFieldPage
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'rtl-and-cjk.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        # `-join` is required, not stylistic: in PowerShell `[char]0x41 + [char]0x42`
        # is 131, because `+` on two chars is numeric addition. Written with `+`
        # this generator dies on the second element.
        $arabic = -join @([char]0x0645, [char]0x0631, [char]0x062D, [char]0x0628, [char]0x0627)
        $hebrew = -join @([char]0x05E9, [char]0x05DC, [char]0x05D5, [char]0x05DD)
        $chinese = -join @([char]0x4F60, [char]0x597D, [char]0x4E16, [char]0x754C)
        $japanese = -join @([char]0x3053, [char]0x3093, [char]0x306B, [char]0x3061, [char]0x306F)
        $lines = @(
            "English baseline text.",
            "$arabic (Arabic)",
            "$hebrew (Hebrew)",
            "$chinese (Chinese)",
            "$japanese (Japanese)",
            "Mixed $arabic inline."
        )
        foreach ($line in $lines) {
            $p = $d.Paragraphs.Add()
            $p.Range.Text = $line
        }
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'lists-numbering.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        foreach ($t in @('First', 'Second', 'Third')) {
            $p = $d.Paragraphs.Add(); $p.Range.Text = $t
        }
        $d.Content.ListFormat.ApplyOutlineNumberDefault()
        $p = $d.Paragraphs.Add(); $p.Range.Text = "Bulleted item"
        $p.Range.ListFormat.ApplyBulletDefault()
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'hyperlinks-bookmarks.docx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "See the target section for detail. Target section here."
        $d.Bookmarks.Add("TargetSection", $d.Range(40, 54)) | Out-Null
        $d.Hyperlinks.Add($d.Range(4, 10), "https://example.com/", "", "", "the target") | Out-Null
        $d.Hyperlinks.Add($d.Range(15, 22), "", "TargetSection", "", "internal link") | Out-Null
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'content-controls.docx' 'Word' $docxDir {
        param($word, $path, $png)
        # Content controls plus data-bound custom XML: the classic silent-loss case.
        $d = $word.Documents.Add()
        $p = $d.Paragraphs.Add(); $p.Range.Text = "Name: "
        $cc = $d.ContentControls.Add(1, $d.Paragraphs.Item($d.Paragraphs.Count).Range)  # wdContentControlText
        $cc.Title = "CustomerName"
        $cc.Tag = "customer-name"
        $p2 = $d.Paragraphs.Add(); $p2.Range.Text = "Date: "
        $d.ContentControls.Add(6, $p2.Range) | Out-Null   # wdContentControlDate
        try {
            $d.CustomXMLParts.Add('<?xml version="1.0"?><customer xmlns="urn:acme:customer"><name>Acme</name></customer>') | Out-Null
        } catch { }
        $d.SaveAs2($path, 12)
        $d.Close(0)
    }

    Invoke-Doc 'template.dotx' 'Word' $docxDir {
        param($word, $path, $png)
        $d = $word.Documents.Add()
        $d.Content.Text = "Template body with a custom style applied."
        $st = $d.Styles.Add("AcmeBodyStyle", 1)   # wdStyleTypeParagraph
        $st.Font.Name = "Georgia"
        $st.Font.Size = 12
        $d.Content.Style = "AcmeBodyStyle"
        $d.SaveAs2($path, 14)   # wdFormatDotx
        $d.Close(0)
    }
} else {
    Write-Host "`nWord: skipped (preflight)" -ForegroundColor Yellow
}

# --------------------------------------------------------------- Excel --------
# Each body runs in a child process and receives ($excel, $path, $png).

if ($doExcel) {
    Write-Host "`nExcel" -ForegroundColor Cyan

    Invoke-Doc 'minimal.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $wb.Worksheets.Item(1).Range("A1").Value2 = "baseline"
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'formulas-basic.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 20; $r++) {
            $ws.Cells.Item($r, 1).Value2 = $r
            $ws.Cells.Item($r, 2).Formula = "=A$r*2"
        }
        $ws.Range("D1").Formula = "=SUM(A1:A20)"
        $ws.Range("D2").Formula = "=AVERAGE(B1:B20)"
        $ws.Range("D3").Formula = '=IF(D1>100,"big","small")'
        $ws.Range("D4").Formula = "=VLOOKUP(5,A1:B20,2,FALSE)"
        $ws.Range("D5").Formula = "=1/0"          # a deliberate #DIV/0!
        $ws.Range("D6").Formula = '=TEXT(TODAY(),"yyyy-mm-dd")'
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'array-formulas.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 10; $r++) {
            $ws.Cells.Item($r, 1).Value2 = $r
            $ws.Cells.Item($r, 2).Value2 = $r * 3
        }
        $ws.Range("D1").FormulaArray = "=SUM(A1:A10*B1:B10)"
        $ws.Range("D3").Formula = "=TRANSPOSE(A1:A10)"   # dynamic array where supported
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'conditional-formatting.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 30; $r++) { $ws.Cells.Item($r, 1).Value2 = ($r * 37) % 100 }
        $rng = $ws.Range("A1:A30")
        $fc = $rng.FormatConditions.Add(1, 5, "50")   # xlCellValue, xlGreater
        $fc.Interior.Color = 13551615
        $rng.FormatConditions.AddDatabar() | Out-Null
        $ws.Range("B1:B30").FormatConditions.AddColorScale(3) | Out-Null
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'data-validation.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $v = $ws.Range("A1:A10").Validation
        $v.Delete()
        $v.Add(3, 1, 1, "Red,Green,Blue")   # xlValidateList
        $v.InputTitle = "Pick a colour"
        $v.ErrorTitle = "Not allowed"
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'merged-frozen-grouped.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1:D1").Merge()
        $ws.Range("A1").Value2 = "Merged title across four columns"
        for ($r = 2; $r -le 40; $r++) { for ($c = 1; $c -le 4; $c++) { $ws.Cells.Item($r, $c).Value2 = "r$r c$c" } }
        $ws.Rows.Item("5:10").Group()
        $ws.Columns.Item("B:C").Group()
        $ws.Activate()
        $ws.Range("B3").Select() | Out-Null
        $excel.ActiveWindow.FreezePanes = $true
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'number-formats.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        # Two parallel arrays rather than an array of pairs: PowerShell's handling
        # of nested arrays through COM produced "Specified cast is not valid".
        $values = @(1234.5678, 0.4271, 45000, 45000, 45000.75, -1234, 0.5)
        $formats = @(
            '#,##0.00',
            '0.00%',
            '[$$-en-US]#,##0.00',
            'yyyy-mm-dd',
            'yyyy-mm-dd hh:mm:ss',
            '#,##0;[Red](#,##0)',
            '# ?/?'
        )
        for ($i = 0; $i -lt $values.Count; $i++) {
            $row = $i + 1
            $ws.Cells.Item($row, 1).Value2 = [double]$values[$i]
            # Applied one at a time so a format this Excel rejects loses only its
            # own row rather than the whole document.
            try { $ws.Cells.Item($row, 1).NumberFormat = [string]$formats[$i] } catch { }
            $ws.Cells.Item($row, 2).Value2 = [string]$formats[$i]
        }
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'defined-names.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1:A5").Value2 = 10
        $wb.Names.Add("WorkbookRate", "=0.15") | Out-Null
        $wb.Names.Add("DataRange", '=Sheet1!$A$1:$A$5') | Out-Null
        # Sheet-scoped name, which shadows a workbook-scoped one of the same name.
        $ws.Names.Add("WorkbookRate", "=0.25") | Out-Null
        $ws.Range("C1").Formula = "=SUM(DataRange)*WorkbookRate"
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'charts.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1").Value2 = "Month"; $ws.Range("B1").Value2 = "Sales"
        for ($r = 2; $r -le 13; $r++) {
            $ws.Cells.Item($r, 1).Value2 = "M$($r-1)"
            $ws.Cells.Item($r, 2).Value2 = 100 + (($r * 73) % 800)
        }
        foreach ($type in @(51, 4, 5)) {   # column, line, pie
            $sh = $ws.Shapes.AddChart2(-1, $type)
            $sh.Chart.SetSourceData($ws.Range("A1:B13"))
        }
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'pivot-table.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1").Value2 = "Region"; $ws.Range("B1").Value2 = "Product"; $ws.Range("C1").Value2 = "Amount"
        $regions = @('North', 'South', 'East', 'West'); $products = @('Widget', 'Gadget', 'Doohickey')
        for ($r = 2; $r -le 121; $r++) {
            $ws.Cells.Item($r, 1).Value2 = [string]$regions[($r) % 4]
            $ws.Cells.Item($r, 2).Value2 = [string]$products[($r) % 3]
            # Explicit [double]: writing an Int32 to this column raised "unable to
            # cast System.Int32 to System.String" through COM, even though the
            # identical assignment works elsewhere in this script.
            $ws.Cells.Item($r, 3).Value2 = [double](10 + (($r * 47) % 490))
        }
        $target = $wb.Worksheets.Add()
        $target.Name = "Pivot"
        # Range objects rather than R1C1 strings: the string forms went through an
        # overload that wanted an integer and failed with a cast error.
        $cache = $wb.PivotCaches().Create(1, $ws.Range("A1:C121"))   # xlDatabase
        $pt = $cache.CreatePivotTable($target.Range("A3"), "SalesPivot")
        $pt.PivotFields("Region").Orientation = 1      # xlRowField
        $pt.PivotFields("Product").Orientation = 2     # xlColumnField
        $pt.AddDataField($pt.PivotFields("Amount"), "Sum of Amount", -4157) | Out-Null   # xlSum
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'multi-sheet-references.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        $wb = $excel.Workbooks.Add()
        $a = $wb.Worksheets.Item(1); $a.Name = "Data"
        $b = $wb.Worksheets.Add(); $b.Name = "Summary"
        for ($r = 1; $r -le 50; $r++) { $a.Cells.Item($r, 1).Value2 = $r }
        $b.Range("A1").Formula = "=SUM(Data!A1:A50)"
        $b.Range("A2").Formula = '=COUNTIF(Data!A1:A50,">25")'
        # A sheet name needing quotes in references is its own parsing hazard.
        $c = $wb.Worksheets.Add(); $c.Name = "Q1 Results"
        $c.Range("A1").Value2 = 7
        $b.Range("A3").Formula = "='Q1 Results'!A1*2"
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }

    Invoke-Doc 'shared-strings-heavy.xlsx' 'Excel' $xlsxDir {
        param($excel, $path, $png)
        # Exercises the sharedStrings dedup path: many cells, few distinct values.
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $states = @('Active', 'Pending', 'Closed', 'Escalated')
        $data = New-Object 'object[,]' 5000, 2
        for ($r = 0; $r -lt 5000; $r++) {
            $data[$r, 0] = $states[$r % 4]
            $data[$r, 1] = $r
        }
        $ws.Range("A1:B5000").Value2 = $data
        $wb.SaveAs($path, 51)
        $wb.Close($false)
    }
} else {
    Write-Host "`nExcel: skipped (preflight)" -ForegroundColor Yellow
}

Stop-StrayOfficeProcesses

# -------------------------------------------------------------- Report --------

# The manifest travels with the files so their provenance is known on the far
# side: which Office produced them, and what this run could not produce.
try {
    $wordExe = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\winword.exe' -ErrorAction Stop).'(default)'
    $script:Versions['Word'] = (Get-Item $wordExe).VersionInfo.ProductVersion
} catch { }
try {
    $excelExe = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe' -ErrorAction Stop).'(default)'
    $script:Versions['Excel'] = (Get-Item $excelExe).VersionInfo.ProductVersion
} catch { }

$manifestPath = Join-Path $OutDir 'manifest.json'
[pscustomobject]@{
    generatedUtc = (Get-Date).ToUniversalTime().ToString('o')
    machine      = $env:COMPUTERNAME
    os           = (Get-CimInstance Win32_OperatingSystem).Caption
    office       = $script:Versions
    generated    = $script:Made
    skipped      = $script:Skipped
} | ConvertTo-Json -Depth 4 | Out-File -FilePath $manifestPath -Encoding utf8

Write-Host "`n----------------------------------------" -ForegroundColor Cyan
Write-Host "generated: $($script:Made.Count)" -ForegroundColor Green
if ($script:Skipped.Count -gt 0) {
    Write-Host "skipped:   $($script:Skipped.Count)" -ForegroundColor Yellow
    $script:Skipped | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
}

if ($script:Made.Count -eq 0) {
    Write-Host "`nNothing was generated." -ForegroundColor Red
    exit 1
}

if (-not $NoZip) {
    $zip = Join-Path $OutDir 'corpus.zip'
    if (Test-Path $zip) { Remove-Item $zip -Force }
    $items = @($docxDir, $xlsxDir, $manifestPath, $script:PngPath) | Where-Object { Test-Path $_ }
    Compress-Archive -Path $items -DestinationPath $zip -Force
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 2)
    Write-Host "`nzip: $zip  ($mb MB)" -ForegroundColor Green
    Write-Host "Send that file back; it is the whole corpus."
} else {
    Write-Host "`nFiles are under $OutDir"
}
