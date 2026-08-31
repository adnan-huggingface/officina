param([string]$DocPath, [int]$Page = 4)
$ErrorActionPreference = "Stop"
$word = New-Object -ComObject Word.Application
$word.Visible = $false
try {
  $document = $word.Documents.Open($DocPath, $false, $true)
  # wdGoToPage=1, wdGoToAbsolute=1
  $start = $document.GoTo(1, 1, $Page).Start
  $end = $document.GoTo(1, 1, $Page + 1).Start
  if ($end -le $start) { $end = $document.Content.End }
  $range = $document.Range($start, $end)
  $i = 0
  foreach ($p in $range.Paragraphs) {
    $i++
    $r = $p.Range.Duplicate
    $r.Collapse(1)
    $y = $r.Information(6)
    $pg = $r.Information(3)
    if ($pg -ne $Page) { continue }
    $text = $p.Range.Text -replace "[\r\n\a]", " "
    if ($text.Length -gt 28) { $text = $text.Substring(0, 28) }
    $f = $p.Range.Font
    $pf = $p.Format
    $inTable = $p.Range.Information(12)
    "{0,2} y={1,7:0.00} tbl={2} font={3} {4}pt rule={5} ls={6:0.00} before={7:0.00} after={8:0.00} | {9}" -f `
      $i, $y, [int]$inTable, $f.Name, $f.Size, $pf.LineSpacingRule, $pf.LineSpacing, `
      $pf.SpaceBefore, $pf.SpaceAfter, $text
  }
} finally {
  if ($document) { $document.Close($false) }
  $word.Quit()
  [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($word)
}
