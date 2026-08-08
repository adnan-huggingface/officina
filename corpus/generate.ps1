<#
.SYNOPSIS
  Generates the fidelity corpus by driving Microsoft Word and Excel through COM.

.DESCRIPTION
  The corpus has to be real Office output — that is the whole point of it. Rather
  than collecting files of uncertain provenance, this drives the real applications
  and asks them to produce documents exercising the features most likely to be
  silently destroyed by a naive round trip.

  This script is self-contained. Copy just this one file to a machine with a
  licensed Word and Excel, run it, and send back the zip it produces. It needs no
  other part of the repository and writes nothing outside its output directory.

  Each document is generated independently. A feature this version of Office does
  not support, or that COM refuses to script, is reported and skipped rather than
  aborting the run.

.PARAMETER OutDir
  Where to write docx/, xlsx/, the manifest, and the zip.
  Defaults to a `corpus-out` folder beside this script.

.PARAMETER NoZip
  Leave the loose files without packaging them.

.PARAMETER SkipPreflight
  Skip the licence check. Only useful if the check misfires on a working install.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\generate.ps1

.NOTES
  Preflight exists because an unlicensed Office ("read and print only mode") does
  not fail cleanly — it opens a modal activation dialog that blocks COM even with
  Visible=$false, so the script hangs forever on the first document. Preflight runs
  one throwaway save in a separate process under a timeout to catch that up front.

  Generated documents are clean by construction. They do not carry the accumulated
  oddities of a document edited across many Word versions, which is where the
  nastiest preservation bugs live. Supplement with real-world files where possible.
#>

[CmdletBinding()]
param(
    [string]$OutDir = (Join-Path $PSScriptRoot 'corpus-out'),
    [switch]$NoZip,
    [switch]$SkipPreflight
)

$ErrorActionPreference = 'Stop'

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
    Stop-StrayOfficeProcesses
    break
}

function Invoke-Doc {
    <#  Runs one generator, isolating its failure from the rest of the run.  #>
    param([string]$Name, [scriptblock]$Body)
    # Printed before the work, not after: a COM call that never returns produces
    # no error and no output, so without this a stall is indistinguishable from
    # slowness and there is nothing to report about which document caused it.
    Write-Host ("  ...     $Name") -ForegroundColor DarkGray -NoNewline
    try {
        & $Body
        $script:Made += $Name
        Write-Host "`r  ok      $Name          " -ForegroundColor Green
    } catch {
        Write-Host "`r" -NoNewline
        $script:Skipped += "$Name  ($($_.Exception.Message))"
        Write-Host "  skipped $Name  -> $($_.Exception.Message)" -ForegroundColor Yellow
    }
}

# ------------------------------------------------------------- Preflight ------

function Test-OfficeScriptable {
    <#
      Saves one throwaway document in a child process under a timeout.

      A timeout means Office is blocked on a dialog we cannot see or dismiss —
      overwhelmingly an unlicensed install. A thrown error means COM refused for
      some other reason. Both are reported rather than left to hang the run.
    #>
    param(
        [string]$ProgId,
        [string]$Extension,
        [int]$SaveFormat,
        [int]$TimeoutSec = 120
    )

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
        'ok'
    } -ArgumentList $ProgId, $probe, $SaveFormat

    $finished = Wait-Job $job -Timeout $TimeoutSec
    $reason = $null

    if (-not $finished) {
        Stop-Job $job -ErrorAction SilentlyContinue
        $reason = "no response after ${TimeoutSec}s — Office is almost certainly showing an activation dialog (unlicensed / 'read and print only mode'). Automation cannot dismiss it."
    } else {
        $out = Receive-Job $job -ErrorAction SilentlyContinue 2>&1
        if (-not (Test-Path $probe)) {
            $reason = "the probe produced no file. $out".Trim()
        }
    }

    Remove-Job $job -Force -ErrorAction SilentlyContinue
    if (Test-Path $probe) { Remove-Item $probe -Force -ErrorAction SilentlyContinue }
    Stop-StrayOfficeProcesses

    return [pscustomobject]@{ Ok = ($null -eq $reason); Reason = $reason }
}

$doWord  = $true
$doExcel = $true

if (-not $SkipPreflight) {
    Write-Host "Preflight (one throwaway save per app; up to 2 min each)" -ForegroundColor Cyan

    $w = Test-OfficeScriptable -ProgId 'Word.Application'  -Extension '.docx' -SaveFormat 12
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
        Write-Host "Run this on a machine with a licensed Office install." -ForegroundColor Red
        exit 1
    }
}

# Random data is seeded so two runs on two machines differ only where Office
# itself differs, which keeps corpus diffs meaningful.
Get-Random -SetSeed 20260808 | Out-Null

# A small PNG for the image-wrapping cases, so the script has no external assets.
function New-SamplePng {
    param([string]$Path)
    if (Test-Path $Path) { return }
    Add-Type -AssemblyName System.Drawing
    $bmp = New-Object System.Drawing.Bitmap 160, 120
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(70, 110, 190))
    $g.FillEllipse([System.Drawing.Brushes]::Gold, 30, 20, 100, 80)
    $g.Dispose()
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

$pngPath = Join-Path $OutDir 'sample-image.png'
New-SamplePng -Path $pngPath

# ---------------------------------------------------------------- Word --------

function Save-Doc {
    param($Doc, [string]$Name, [int]$Format = 12)
    $path = Join-Path $docxDir $Name
    if (Test-Path $path) { Remove-Item $path -Force }
    $Doc.SaveAs2($path, $Format)
    $Doc.Close(0)
}

function Invoke-WordCorpus {
    Write-Host "`nWord" -ForegroundColor Cyan
    $script:word = New-Object -ComObject Word.Application
    $script:word.Visible = $false
    $script:word.DisplayAlerts = 0   # wdAlertsNone
    $word = $script:word

    $script:Versions['Word'] = "$($word.Version) build $($word.Build)"

    $wdFormatDotx = 14

    Invoke-Doc 'minimal.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "A single paragraph. The baseline case."
        Save-Doc $d 'minimal.docx'
    }

    Invoke-Doc 'styles-headings-toc.docx' {
        $d = $word.Documents.Add()
        foreach ($h in @(@('Introduction',1), @('Background',2), @('Method',2), @('Results',1))) {
            $p = $d.Paragraphs.Add()
            $p.Range.Text = $h[0]
            $p.Range.Style = "Heading $($h[1])"
            $body = $d.Paragraphs.Add()
            $body.Range.Text = "Body text under $($h[0]). " * 4
            $body.Range.Style = 'Normal'
        }
        # A TOC field: fields are a classic thing to lose on round trip.
        $d.TablesOfContents.Add($d.Range(0,0), $true, 1, 3) | Out-Null
        Save-Doc $d 'styles-headings-toc.docx'
    }

    Invoke-Doc 'tracked-changes.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "The original sentence stays here. This sentence will be deleted."
        $d.TrackRevisions = $true
        $p = $d.Paragraphs.Add()
        $p.Range.Text = "This paragraph was inserted with revision tracking on."
        # Delete a stretch so the file carries both an insertion and a deletion.
        $d.Range(35, 64).Delete() | Out-Null
        $d.TrackRevisions = $false
        Save-Doc $d 'tracked-changes.docx'
    }

    Invoke-Doc 'comments.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "A sentence that reviewers argued about at some length."
        $c = $d.Comments.Add($d.Range(2, 10), "Is this the right word?")
        # A reply, which is stored differently from a top-level comment.
        try { $c.Replies.Add($c.Range, "I think so.") | Out-Null } catch { }
        Save-Doc $d 'comments.docx'
    }

    Invoke-Doc 'nested-tables.docx' {
        $d = $word.Documents.Add()
        $outer = $d.Tables.Add($d.Range(0,0), 3, 3)
        $outer.Borders.Enable = $true
        for ($r = 1; $r -le 3; $r++) {
            for ($c = 1; $c -le 3; $c++) { $outer.Cell($r,$c).Range.Text = "R$r C$c" }
        }
        # Table inside a cell: layout code that assumes a flat table breaks here.
        $inner = $d.Tables.Add($outer.Cell(2,2).Range, 2, 2)
        $inner.Borders.Enable = $true
        $inner.Cell(1,1).Range.Text = "inner"
        Save-Doc $d 'nested-tables.docx'
    }

    Invoke-Doc 'table-spanning-pages.docx' {
        $d = $word.Documents.Add()
        $t = $d.Tables.Add($d.Range(0,0), 60, 4)
        $t.Borders.Enable = $true
        $t.Rows.Item(1).HeadingFormat = $true   # repeats on each page
        for ($c = 1; $c -le 4; $c++) { $t.Cell(1,$c).Range.Text = "Header $c" }
        for ($r = 2; $r -le 60; $r++) {
            for ($c = 1; $c -le 4; $c++) { $t.Cell($r,$c).Range.Text = "r$r-c$c" }
        }
        Save-Doc $d 'table-spanning-pages.docx'
    }

    Invoke-Doc 'floating-image-wrap.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = ("Text that must flow around the floating image. " * 40)
        $shape = $d.Shapes.AddPicture($pngPath, $false, $true, 100, 100, 160, 120)
        $shape.WrapFormat.Type = 2   # wdWrapSquare
        # An inline image too: inline and floating are different storage entirely.
        $d.InlineShapes.AddPicture($pngPath, $false, $true, $d.Range(0,0)) | Out-Null
        Save-Doc $d 'floating-image-wrap.docx'
    }

    Invoke-Doc 'footnotes-endnotes.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "A claim requiring a footnote. And another requiring an endnote."
        $d.Footnotes.Add($d.Range(30, 30), "", "The supporting footnote text.") | Out-Null
        $d.Endnotes.Add($d.Range(60, 60), "", "The supporting endnote text.") | Out-Null
        Save-Doc $d 'footnotes-endnotes.docx'
    }

    Invoke-Doc 'sections-mixed-orientation.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "Portrait section."
        $d.Sections.Add() | Out-Null
        $d.Sections.Item(2).PageSetup.Orientation = 1   # wdOrientLandscape
        $d.Sections.Item(2).Range.InsertAfter("Landscape section with different page setup.")
        $d.Sections.Add() | Out-Null
        $d.Sections.Item(3).PageSetup.Orientation = 0
        $d.Sections.Item(3).PageSetup.LeftMargin = 144
        $d.Sections.Item(3).Range.InsertAfter("Third section, wider margins.")
        Save-Doc $d 'sections-mixed-orientation.docx'
    }

    Invoke-Doc 'headers-footers.docx' {
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
        Save-Doc $d 'headers-footers.docx'
    }

    Invoke-Doc 'rtl-and-cjk.docx' {
        $d = $word.Documents.Add()
        $lines = @(
            "English baseline text.",
            [char]0x0645 + [char]0x0631 + [char]0x062D + [char]0x0628 + [char]0x0627 + " (Arabic)",
            [char]0x05E9 + [char]0x05DC + [char]0x05D5 + [char]0x05DD + " (Hebrew)",
            [char]0x4F60 + [char]0x597D + [char]0x4E16 + [char]0x754C + " (Chinese)",
            [char]0x3053 + [char]0x3093 + [char]0x306B + [char]0x3061 + [char]0x306F + " (Japanese)",
            "Mixed " + [char]0x0645 + [char]0x0631 + [char]0x062D + [char]0x0628 + [char]0x0627 + " inline."
        )
        foreach ($line in $lines) {
            $p = $d.Paragraphs.Add()
            $p.Range.Text = $line
        }
        Save-Doc $d 'rtl-and-cjk.docx'
    }

    Invoke-Doc 'lists-numbering.docx' {
        $d = $word.Documents.Add()
        foreach ($t in @('First','Second','Third')) {
            $p = $d.Paragraphs.Add(); $p.Range.Text = $t
        }
        $d.Content.ListFormat.ApplyOutlineNumberDefault()
        $p = $d.Paragraphs.Add(); $p.Range.Text = "Bulleted item"
        $p.Range.ListFormat.ApplyBulletDefault()
        Save-Doc $d 'lists-numbering.docx'
    }

    Invoke-Doc 'hyperlinks-bookmarks.docx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "See the target section for detail. Target section here."
        $d.Bookmarks.Add("TargetSection", $d.Range(40, 54)) | Out-Null
        $d.Hyperlinks.Add($d.Range(4, 10), "https://example.com/", "", "", "the target") | Out-Null
        $d.Hyperlinks.Add($d.Range(15, 22), "", "TargetSection", "", "internal link") | Out-Null
        Save-Doc $d 'hyperlinks-bookmarks.docx'
    }

    Invoke-Doc 'content-controls.docx' {
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
        Save-Doc $d 'content-controls.docx'
    }

    Invoke-Doc 'template.dotx' {
        $d = $word.Documents.Add()
        $d.Content.Text = "Template body with a custom style applied."
        $st = $d.Styles.Add("AcmeBodyStyle", 1)   # wdStyleTypeParagraph
        $st.Font.Name = "Georgia"
        $st.Font.Size = 12
        $d.Content.Style = "AcmeBodyStyle"
        Save-Doc $d 'template.dotx' $wdFormatDotx
    }

    $word.Quit()
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
    $script:word = $null
}

# --------------------------------------------------------------- Excel --------

function Save-Book {
    param($Book, [string]$Name, [int]$Format = 51)
    $path = Join-Path $xlsxDir $Name
    if (Test-Path $path) { Remove-Item $path -Force }
    $Book.SaveAs($path, $Format)
    $Book.Close($false)
}

function Invoke-ExcelCorpus {
    Write-Host "`nExcel" -ForegroundColor Cyan
    $script:excel = New-Object -ComObject Excel.Application
    $script:excel.Visible = $false
    $script:excel.DisplayAlerts = $false
    $excel = $script:excel

    $script:Versions['Excel'] = "$($excel.Version) build $($excel.Build)"

    Invoke-Doc 'minimal.xlsx' {
        $wb = $excel.Workbooks.Add()
        $wb.Worksheets.Item(1).Range("A1").Value2 = "baseline"
        Save-Book $wb 'minimal.xlsx'
    }

    Invoke-Doc 'formulas-basic.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 20; $r++) {
            $ws.Cells.Item($r, 1).Value2 = $r
            $ws.Cells.Item($r, 2).Formula = "=A$r*2"
        }
        $ws.Range("D1").Formula = "=SUM(A1:A20)"
        $ws.Range("D2").Formula = "=AVERAGE(B1:B20)"
        $ws.Range("D3").Formula = "=IF(D1>100,`"big`",`"small`")"
        $ws.Range("D4").Formula = "=VLOOKUP(5,A1:B20,2,FALSE)"
        $ws.Range("D5").Formula = "=1/0"          # a deliberate #DIV/0!
        $ws.Range("D6").Formula = "=TEXT(TODAY(),`"yyyy-mm-dd`")"
        Save-Book $wb 'formulas-basic.xlsx'
    }

    Invoke-Doc 'array-formulas.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 10; $r++) {
            $ws.Cells.Item($r,1).Value2 = $r
            $ws.Cells.Item($r,2).Value2 = $r * 3
        }
        $ws.Range("D1").FormulaArray = "=SUM(A1:A10*B1:B10)"
        $ws.Range("D3").Formula = "=TRANSPOSE(A1:A10)"   # dynamic array where supported
        Save-Book $wb 'array-formulas.xlsx'
    }

    Invoke-Doc 'conditional-formatting.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        for ($r = 1; $r -le 30; $r++) { $ws.Cells.Item($r,1).Value2 = (Get-Random -Min 0 -Max 100) }
        $rng = $ws.Range("A1:A30")
        $fc = $rng.FormatConditions.Add(1, 5, "50")   # xlCellValue, xlGreater
        $fc.Interior.Color = 13551615
        $rng.FormatConditions.AddDatabar() | Out-Null
        $ws.Range("B1:B30").FormatConditions.AddColorScale(3) | Out-Null
        Save-Book $wb 'conditional-formatting.xlsx'
    }

    Invoke-Doc 'data-validation.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $v = $ws.Range("A1:A10").Validation
        $v.Delete()
        $v.Add(3, 1, 1, "Red,Green,Blue")   # xlValidateList
        $v.InputTitle = "Pick a colour"
        $v.ErrorTitle = "Not allowed"
        Save-Book $wb 'data-validation.xlsx'
    }

    Invoke-Doc 'merged-frozen-grouped.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1:D1").Merge()
        $ws.Range("A1").Value2 = "Merged title across four columns"
        for ($r = 2; $r -le 40; $r++) { for ($c = 1; $c -le 4; $c++) { $ws.Cells.Item($r,$c).Value2 = "r$r c$c" } }
        $ws.Rows.Item("5:10").Group()
        $ws.Columns.Item("B:C").Group()
        $excel.ActiveWindow.FreezePanes = $false
        $ws.Activate()
        $ws.Range("B3").Select() | Out-Null
        $excel.ActiveWindow.FreezePanes = $true
        Save-Book $wb 'merged-frozen-grouped.xlsx'
    }

    Invoke-Doc 'number-formats.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $pairs = @(
            @(1234.5678, '#,##0.00'),
            @(0.4271,    '0.00%'),
            @(45000,     '[$$-en-US]#,##0.00'),
            @(45000,     'yyyy-mm-dd'),
            @(45000.75,  'yyyy-mm-dd hh:mm:ss'),
            @(-1234,     '#,##0;[Red](#,##0)'),
            @(0.5,       '# ?/?')
        )
        for ($i = 0; $i -lt $pairs.Count; $i++) {
            $ws.Cells.Item($i+1, 1).Value2 = $pairs[$i][0]
            $ws.Cells.Item($i+1, 1).NumberFormat = $pairs[$i][1]
            $ws.Cells.Item($i+1, 2).Value2 = $pairs[$i][1]
        }
        Save-Book $wb 'number-formats.xlsx'
    }

    Invoke-Doc 'defined-names.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1:A5").Value2 = 10
        $wb.Names.Add("WorkbookRate", "=0.15") | Out-Null
        $wb.Names.Add("DataRange", "=Sheet1!`$A`$1:`$A`$5") | Out-Null
        # Sheet-scoped name, which shadows a workbook-scoped one of the same name.
        $ws.Names.Add("WorkbookRate", "=0.25") | Out-Null
        $ws.Range("C1").Formula = "=SUM(DataRange)*WorkbookRate"
        Save-Book $wb 'defined-names.xlsx'
    }

    Invoke-Doc 'charts.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1").Value2 = "Month"; $ws.Range("B1").Value2 = "Sales"
        for ($r = 2; $r -le 13; $r++) {
            $ws.Cells.Item($r,1).Value2 = "M$($r-1)"
            $ws.Cells.Item($r,2).Value2 = (Get-Random -Min 100 -Max 900)
        }
        foreach ($type in @(51, 4, 5)) {   # column, line, pie
            $sh = $ws.Shapes.AddChart2(-1, $type)
            $sh.Chart.SetSourceData($ws.Range("A1:B13"))
        }
        Save-Book $wb 'charts.xlsx'
    }

    Invoke-Doc 'pivot-table.xlsx' {
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $ws.Range("A1").Value2 = "Region"; $ws.Range("B1").Value2 = "Product"; $ws.Range("C1").Value2 = "Amount"
        $regions = @('North','South','East','West'); $products = @('Widget','Gadget','Doohickey')
        for ($r = 2; $r -le 121; $r++) {
            $ws.Cells.Item($r,1).Value2 = $regions[($r) % 4]
            $ws.Cells.Item($r,2).Value2 = $products[($r) % 3]
            $ws.Cells.Item($r,3).Value2 = (Get-Random -Min 10 -Max 500)
        }
        $target = $wb.Worksheets.Add()
        $target.Name = "Pivot"
        $cache = $wb.PivotCaches().Create(1, "Sheet1!R1C1:R121C3")   # xlDatabase
        $pt = $cache.CreatePivotTable("Pivot!R3C1", "SalesPivot")
        $pt.PivotFields("Region").Orientation = 1      # xlRowField
        $pt.PivotFields("Product").Orientation = 2     # xlColumnField
        $pt.AddDataField($pt.PivotFields("Amount"), "Sum of Amount", -4157) | Out-Null   # xlSum
        Save-Book $wb 'pivot-table.xlsx'
    }

    Invoke-Doc 'multi-sheet-references.xlsx' {
        $wb = $excel.Workbooks.Add()
        $a = $wb.Worksheets.Item(1); $a.Name = "Data"
        $b = $wb.Worksheets.Add(); $b.Name = "Summary"
        for ($r = 1; $r -le 50; $r++) { $a.Cells.Item($r,1).Value2 = $r }
        $b.Range("A1").Formula = "=SUM(Data!A1:A50)"
        $b.Range("A2").Formula = "=COUNTIF(Data!A1:A50,`">25`")"
        # A sheet name needing quotes in references is its own parsing hazard.
        $c = $wb.Worksheets.Add(); $c.Name = "Q1 Results"
        $c.Range("A1").Value2 = 7
        $b.Range("A3").Formula = "='Q1 Results'!A1*2"
        Save-Book $wb 'multi-sheet-references.xlsx'
    }

    Invoke-Doc 'shared-strings-heavy.xlsx' {
        # Exercises the sharedStrings dedup path: many cells, few distinct values.
        $wb = $excel.Workbooks.Add()
        $ws = $wb.Worksheets.Item(1)
        $states = @('Active','Pending','Closed','Escalated')
        $data = New-Object 'object[,]' 5000,2
        for ($r = 0; $r -lt 5000; $r++) {
            $data[$r,0] = $states[$r % 4]
            $data[$r,1] = $r
        }
        $ws.Range("A1:B5000").Value2 = $data
        Save-Book $wb 'shared-strings-heavy.xlsx'
    }

    $excel.Quit()
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($excel) | Out-Null
    $script:excel = $null
}

# ----------------------------------------------------------------- Run --------

if ($doWord)  { Invoke-WordCorpus }  else { Write-Host "`nWord: skipped (preflight)"  -ForegroundColor Yellow }
if ($doExcel) { Invoke-ExcelCorpus } else { Write-Host "`nExcel: skipped (preflight)" -ForegroundColor Yellow }

[GC]::Collect(); [GC]::WaitForPendingFinalizers()
Stop-StrayOfficeProcesses

# -------------------------------------------------------------- Report --------

# The manifest travels with the files so their provenance is known on the far side:
# which Office produced them, and what this run could not produce.
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
    $items = @($docxDir, $xlsxDir, $manifestPath, $pngPath) | Where-Object { Test-Path $_ }
    Compress-Archive -Path $items -DestinationPath $zip -Force
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 2)
    Write-Host "`nzip: $zip  ($mb MB)" -ForegroundColor Green
    Write-Host "Send that file back; it is the whole corpus."
} else {
    Write-Host "`nFiles are under $OutDir"
}
