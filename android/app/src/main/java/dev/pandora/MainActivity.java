package dev.pandora;

import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.view.View;
import android.view.WindowManager;

import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

/**
 * The one activity. It loads the Rust library, draws edge to edge behind the
 * system bars, and hands the file picker's answer back to {@link Pandora} —
 * which is the surface the native side talks to.
 *
 * <p>GameActivity rather than a plain Activity because it is the class
 * `android-activity` and `eframe` expect, and it brings an input method
 * connection, which the master-password field needs.
 */
public class MainActivity extends GameActivity {

    static {
        // The `.so` `cargo ndk` drops into jniLibs, and what
        // `android.app.lib_name` names in the manifest.
        System.loadLibrary("pandora");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Before `super`, which is where the native side starts: GameActivity
        // spawns the thread `android_main` builds the app on inside that call, and
        // the app reads a remembered session from the platform as it boots. The
        // handle has to be in place before that thread exists.
        Pandora.attach(this);
        // The vault's contents must never leave the screen: no screenshots, and
        // no thumbnail in the recents/task manager either.
        getWindow().setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE);
        // The app draws its own chrome and uses the whole screen, cutouts
        // included — but the phone's navigation bar stays on show: it is the way
        // out of the app, and the bar at the bottom leaves it exactly its room.
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        systemBars();
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void onResume() {
        super.onResume();
        Pandora.resumed();
        systemBars();
    }

    @Override
    protected void onPause() {
        Pandora.paused();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        Pandora.detach(this);
        super.onDestroy();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        Pandora.picked(requestCode, resultCode, data);
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] granted) {
        super.onRequestPermissionsResult(requestCode, permissions, granted);
        Pandora.asked(requestCode, granted);
    }

    /**
     * The status bar and the cutout go away; the navigation bar stays. The app's
     * own bar reserves its height ({@link Pandora#footer}), so nothing of the app
     * ever sits under it.
     */
    private void systemBars() {
        // `SHORT_EDGES` and not `ALWAYS`: the latter is API 30, and a phone that
        // never asked for it would have an attribute it cannot read.
        getWindow().getAttributes().layoutInDisplayCutoutMode =
                WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        View decor = getWindow().getDecorView();
        WindowInsetsControllerCompat controller =
                new WindowInsetsControllerCompat(getWindow(), decor);
        controller.hide(WindowInsetsCompat.Type.statusBars());
        controller.hide(WindowInsetsCompat.Type.displayCutout());
        // The theme is fullscreen, so the navigation bar has to be asked back:
        // hidden, it would only come out on a swipe, which is the one thing this
        // app does not want over its own bottom bar.
        controller.show(WindowInsetsCompat.Type.navigationBars());
        controller.setSystemBarsBehavior(
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }
}
