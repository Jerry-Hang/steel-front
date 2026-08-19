
using System;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public static class DSHHelper
{
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static bool SendToDsh(string message)
    {
        // 找 Edge DeepSeek 窗口
        IntPtr hwnd = IntPtr.Zero;
        foreach (var p in System.Diagnostics.Process.GetProcessesByName("msedge"))
        {
            if (p.MainWindowHandle != IntPtr.Zero && p.MainWindowTitle.Contains("DeepSeek"))
            {
                hwnd = p.MainWindowHandle;
                break;
            }
        }
        if (hwnd == IntPtr.Zero) return false;
        SetForegroundWindow(hwnd);
        System.Threading.Thread.Sleep(900);
        RECT rect;
        if (!GetWindowRect(hwnd, out rect)) return false;
        int w = rect.Right - rect.Left;
        int h = rect.Bottom - rect.Top;
        // 点击输入框（底部中央）
        SetCursorPos(rect.Left + w / 2, rect.Top + h - 45);
        System.Threading.Thread.Sleep(400);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(800);
        SendKeys.SendWait(message);
        System.Threading.Thread.Sleep(600);
        SendKeys.SendWait("~");
        return true;
    }

    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
