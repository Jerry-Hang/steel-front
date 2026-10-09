package com.steelfront;

import com.google.androidgamesdk.GameActivity;

/**
 * 薄壳：GameActivity 负责加载 libsteel_front.so（名字来自 AndroidManifest 的
 * android.app.lib_name）并在其线程上回调 Rust 侧的 android_main。
 *
 * 本类不写任何逻辑——引擎全在 Rust 里。留这个类只是为了满足 GameActivity
 * 需要一个子类作为 Activity 的约定。
 */
public class MainActivity extends GameActivity {
}
