package dev.pandora;

import android.content.Context;

/**
 * The native half of autofill, one string at a time.
 *
 * <p>{@link AutofillService} asks; the Rust side answers out of the copy of the
 * open box it was handed. Nothing here holds a secret, and nothing here parses
 * the box: {@code nativeFill} is the one call that brings a password back, and
 * only for the id the user picked in the panel.
 */
final class Autofill {

    private Autofill() {
    }

    static {
        // The same `.so` `MainActivity` loads. A fill request can start the
        // process, so this class may be the first thing that asks for it.
        System.loadLibrary("pandora");
    }

    /** The logger, and the VM and context the service needs in a process with no activity. */
    static native void nativeInit(Context context);

    /** {@code "ready"} while a box is open here, {@code "locked"} otherwise. */
    static native String nativeState();

    /** Drop the copy now: the screen went off, or the box locked. */
    static native void nativeForget();

    /** The entries claiming a page, as JSON, or null. */
    static native String nativeQuery(String url);

    /** {@code {"username","password"}} for one id, or null. */
    static native String nativeFill(String id);

    /** {@code {"code","remaining"}} for one seed, or null. */
    static native String nativeCode(String id);

    /** Every seed by name, as JSON, or null. Never a seed and never a code. */
    static native String nativeCodes();

    /** Queue a login the panel saved: {@code "queued"}, or null when no box is open. */
    static native String nativeSave(String name, String username, String password, String url);
}
