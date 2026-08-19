
using System;
using System.Threading.Tasks;
using Windows.Media.Ocr;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Streams;

public static class OcrHelper
{
    public static string Recognize(string path)
    {
        try
        {
            var file = StorageFile.GetFileFromPathAsync(path).AsTask().Result;
            using (var stream = file.OpenAsync(FileAccessMode.Read).AsTask().Result)
            {
                var decoder = BitmapDecoder.CreateAsync(stream).AsTask().Result;
                var bitmap = decoder.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied).AsTask().Result;
                var engine = OcrEngine.TryCreateFromLanguage(new Windows.Globalization.Language("zh-Hans-CN"));
                if (engine == null) return "[NO_ENGINE]";
                var result = engine.RecognizeAsync(bitmap).AsTask().Result;
                var sb = new System.Text.StringBuilder();
                sb.AppendLine("LINES=" + result.Lines.Count);
                foreach (var line in result.Lines)
                {
                    int y = (int)line.Words[0].BoundingRect.Y;
                    var sb2 = new System.Text.StringBuilder();
                    foreach (var w in line.Words) sb2.Append(w.Text);
                    sb.AppendLine("[" + y + "] " + sb2.ToString());
                }
                return sb.ToString();
            }
        }
        catch (Exception ex)
        {
            return "[EXC] " + ex.Message;
        }
    }
}
