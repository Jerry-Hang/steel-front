
[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics, ContentType = WindowsRuntime] | Out-Null
[Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime] | Out-Null
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTaskT = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 -and $_.ReturnType.Name -like 'Task*' } | Select-Object -First 1
function AwaitOp($op, $t) {
  $m = $asTaskT.MakeGenericMethod($t)
  $task = $m.Invoke($null, @($op))
  $task.Wait() | Out-Null
  $task.Result
}
try {
  $file = AwaitOp ([Windows.Storage.StorageFile]::GetFileFromPathAsync('D:\Rust\steel-front\scripts\ocr_test.png')) ([Windows.Storage.StorageFile])
  $stream = AwaitOp ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
  $decoder = AwaitOp ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
  $bitmap = AwaitOp ($decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied)) ([Windows.Graphics.Imaging.SoftwareBitmap])
  Write-Output ('bitmap: ' + $bitmap.PixelWidth + 'x' + $bitmap.PixelHeight)
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::new('zh-Hans-CN'))
  if (-not $engine) { Write-Output 'engine null'; exit }
  $result = AwaitOp ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
  Write-Output ('OCR lines: ' + $result.Lines.Count)
  foreach ($line in $result.Lines) { Write-Output (($line.Words | ForEach-Object { $_.Text }) -join '') }
} catch {
  Write-Output ('EXC: ' + $_.Exception.Message)
}
