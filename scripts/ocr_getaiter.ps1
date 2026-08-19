
[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics, ContentType = WindowsRuntime] | Out-Null
[Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime] | Out-Null
try {
  $file = [Windows.Storage.StorageFile]::GetFileFromPathAsync('D:\Rust\steel-front\scripts\ocr_test.png').GetAwaiter().GetResult()
  $stream = $file.OpenAsync([Windows.Storage.FileAccessMode]::Read).GetAwaiter().GetResult()
  $decoder = [Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream).GetAwaiter().GetResult()
  $bitmap = $decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied).GetAwaiter().GetResult()
  Write-Output ('bitmap: ' + $bitmap.PixelWidth + 'x' + $bitmap.PixelHeight)
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::new('zh-Hans-CN'))
  if (-not $engine) { Write-Output 'engine null'; exit }
  $result = $engine.RecognizeAsync($bitmap).GetAwaiter().GetResult()
  Write-Output ('OCR lines: ' + $result.Lines.Count)
  foreach ($line in $result.Lines) {
    $txt = ($line.Words | ForEach-Object { $_.Text }) -join ''
    Write-Output ('[' + [int]$line.Words[0].BoundingRect.Y + '] ' + $txt)
  }
} catch {
  Write-Output ('EXC: ' + $_.Exception.Message)
}
