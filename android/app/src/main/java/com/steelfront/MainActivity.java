package com.steelfront;

import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Display;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import com.google.androidgamesdk.GameActivity;

/**
 * 薄壳：GameActivity 负责加载 libsteel_front.so（名字来自 AndroidManifest 的
 * android.app.lib_name）并在其线程上回调 Rust 侧的 android_main。
 *
 * 本类做两件事：
 *
 * ① **进沉浸式全屏**（2026-10-09 实机排查补）：不隐藏系统栏时，横屏下导航栏占 48px、
 *    挖孔 letterbox 占 120px，游戏 surface 只有 2280x1032 而不是 2400x1080
 *    —— 就是实机看到的「黑边」。
 *
 * ② **请求最高刷新率**（2026-10-11 实机排查补）：见 `requestHighestRefreshRate`。
 */
public class MainActivity extends GameActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        hideSystemBars();
        requestHighestRefreshRate();
    }

    /**
     * 请求屏幕支持的最高刷新率。
     *
     * 🔴 为什么需要（2026-10-11 实测）：Android 默认给**不声明首选帧率**的应用 60Hz。
     * 本机（OPPO PLD110）实测：
     * ```
     * activeMode={id=1, resolution=1080x2400, vsyncRate=60.00 Hz}   ← 显示停在 60
     * 引擎日志: frame-cap: 300 fps (focused=true)                    ← 引擎没限帧
     * ```
     * ⇒ 而交换链用 `FIFO` 呈现（= 同步到 vsync）⇒ **游戏被死死封在 60 fps**，
     * 而这块屏支持 **120Hz**（`peak_refresh_rate=120`）—— **白白少一半**。
     *
     * 症状极具迷惑性：内部渲染分辨率三档（2000×900 / 1600×720 / 1200×540）测出来
     * 全是 59.8~59.9，看起来像"降到 1/2 分辨率也没用"；**其实是撞在 60Hz 上**，
     * 三档的真实能力都超出了屏幕刷新率。
     *
     * 修法：把窗口的 `preferredDisplayModeId` 指到屏幕支持的最高刷新率那一档。
     * 失败不影响游戏（catch 住只记日志）—— 低端机可能没有更高的档。
     */
    private void requestHighestRefreshRate() {
        try {
            WindowManager wm = getSystemService(WindowManager.class);
            if (wm == null) {
                return;
            }
            Display display = wm.getDefaultDisplay();
            if (display == null) {
                return;
            }
            Display.Mode best = null;
            for (Display.Mode m : display.getSupportedModes()) {
                if (best == null || m.getRefreshRate() > best.getRefreshRate()) {
                    best = m;
                }
            }
            if (best == null) {
                return;
            }
            WindowManager.LayoutParams lp = getWindow().getAttributes();
            lp.preferredDisplayModeId = best.getModeId();
            getWindow().setAttributes(lp);
            Log.i("steel_front", "请求最高刷新率: " + best.getRefreshRate()
                    + " Hz (modeId=" + best.getModeId()
                    + ", 分辨率=" + best.getPhysicalWidth() + "x" + best.getPhysicalHeight() + ")");
        } catch (Throwable t) {
            Log.w("steel_front", "刷新率请求失败（不影响游戏）", t);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    private void hideSystemBars() {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsetsController c = getWindow().getInsetsController();
                if (c != null) {
                    c.hide(WindowInsets.Type.systemBars());
                    c.setSystemBarsBehavior(
                            WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                }
            } else {
                getWindow().getDecorView().setSystemUiVisibility(
                        View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                                | View.SYSTEM_UI_FLAG_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
            }
        } catch (Throwable t) {
            android.util.Log.w("steel_front", "沉浸式设置失败（不影响游戏）", t);
        }
    }
}
