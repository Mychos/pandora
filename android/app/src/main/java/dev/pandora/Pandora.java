package dev.pandora;

import android.Manifest;
import android.app.Activity;
import android.app.KeyguardManager;
import android.content.BroadcastReceiver;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.ImageFormat;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.media.Image;
import android.media.ImageReader;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.PersistableBundle;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import android.util.Log;
import android.util.Size;

import androidx.core.app.ActivityCompat;

import androidx.biometric.BiometricManager;
import androidx.biometric.BiometricPrompt;
import androidx.browser.customtabs.CustomTabsIntent;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.fragment.app.FragmentActivity;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.Key;
import java.security.KeyStore;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicBoolean;

import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Everything the Rust side cannot do for itself: the keystore key a fingerprint
 * unlocks, the file picker, the clipboard, the browser, and the GitHub token.
 *
 * <p>Static, strings in and strings out, so `src/shared/android.rs` reaches each
 * of these with one JNI signature. All the platform work happens here, and none
 * of it touches the vault: what comes back to Rust is a path, a base64 blob or a
 * copied string.
 *
 * <p>The fingerprint itself never reaches the native side. A fingerprint is not
 * key material — the only thing it can do is let the keystore use a key, and that
 * key never leaves the keystore either. What crosses this boundary is the
 * ciphertext of a box's derived key, going in and coming out.
 */
public final class Pandora {

    private Pandora() {
    }

    /** The keystore key every box's seal is wrapped in. One per device, not per box. */
    private static final String KEY = "pandora.vault";

    /** The keystore key the GitHub token is wrapped in. Needs no finger, so it comes back at boot. */
    private static final String TOKEN_KEY = "pandora.github";

    /** Where the token's ciphertext lives, in the app's own private preferences. */
    private static final String PREFS = "pandora";

    /// The tag `adb logcat -s pandora` filters on: the Rust side's
    /// `android_logger` writes under it, and so does everything here.
    private static final String TAG = "pandora";

    private static final String TOKEN = "github";

    private static final String CIPHER = "AES/GCM/NoPadding";

    private static final int REQUEST_PICK = 1;
    private static final int REQUEST_CAMERA = 3;

    private static volatile Activity activity;
    private static volatile boolean foreground;
    private static final AtomicBoolean SYSTEM_LOCK = new AtomicBoolean();
    private static final AtomicBoolean RESUMED = new AtomicBoolean();
    private static boolean screenReceiver;
    private static final BroadcastReceiver SCREEN = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            if (Intent.ACTION_SCREEN_OFF.equals(intent.getAction())) {
                SYSTEM_LOCK.set(true);
                // The box locks on the next frame, which a phone in a pocket may
                // never paint; the autofill copy must not outlive the screen.
                Autofill.nativeForget();
            }
        }
    };

    /** Why the last answer is missing, when it was a fault and not the user backing out. */
    private static volatile String failure;

    /** Set while a picker or the camera is up, so the answer knows where to land. */
    private static volatile CountDownLatch picking;

    private static volatile String picked;

    /** The camera while a scan is up: device, session, reader and newest frame. */
    private static volatile CameraDevice camera;
    private static volatile CameraCaptureSession session;
    private static volatile ImageReader reader;
    private static volatile byte[] frames;
    private static volatile int frameWidth;
    private static volatile int frameHeight;
    private static volatile int frameTurn;
    private static volatile CountDownLatch opening;
    private static volatile CountDownLatch asking;
    private static volatile String cameraError;
    private static volatile HandlerThread worker;
    private static volatile Handler onWorker;

    static void attach(Activity current) {
        activity = current;
        IntentFilter filter = new IntentFilter(Intent.ACTION_SCREEN_OFF);
        if (Build.VERSION.SDK_INT >= 33) {
            current.registerReceiver(SCREEN, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            current.registerReceiver(SCREEN, filter);
        }
        screenReceiver = true;
    }

    static void detach(Activity leaving) {
        if (activity == leaving) {
            if (screenReceiver) {
                leaving.unregisterReceiver(SCREEN);
                screenReceiver = false;
            }
            activity = null;
            foreground = false;
        }
    }

    static void resumed() {
        foreground = true;
        RESUMED.set(true);
        Activity current = activity;
        if (current != null) {
            KeyguardManager keyguard = (KeyguardManager) current.getSystemService(Context.KEYGUARD_SERVICE);
            if (keyguard != null && keyguard.isKeyguardLocked()) {
                SYSTEM_LOCK.set(true);
            }
        }
    }

    static void paused() {
        foreground = false;
        // A camera nobody is looking at is a battery drain and, worse, a light on.
        stopCamera();
    }

    static boolean foreground() {
        return foreground;
    }

    static boolean takeSystemLock() {
        return SYSTEM_LOCK.getAndSet(false);
    }

    static boolean takeResumed() {
        return RESUMED.getAndSet(false);
    }

    /** Is there a fingerprint reader this app is allowed to use? */
    static boolean available() {
        Activity current = activity;
        if (current == null) {
            return false;
        }
        return BiometricManager.from(current)
                .canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                == BiometricManager.BIOMETRIC_SUCCESS;
    }

    /**
     * The phone's own navigation bar at the bottom, in dp — the app's bar stands
     * on top of it instead of under it.
     *
     * <p>The activity draws edge to edge, so the framework does not inset the
     * window for the system bars and keeping clear of them is the app's own job.
     * The *visible* insets: that bar is never hidden, so what it reports is what is
     * really there — and it moves with the phone, which is why this is asked for
     * again rather than remembered.
     */
    static float footer() {
        Activity current = activity;
        if (current == null) {
            return 0f;
        }
        WindowInsetsCompat insets =
                ViewCompat.getRootWindowInsets(current.getWindow().getDecorView());
        if (insets == null) {
            return 0f;
        }
        int bottom = insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom;
        return bottom / current.getResources().getDisplayMetrics().density;
    }

    /**
     * Wrap a box's derived key under the keystore key. A finger is asked for here
     * as well: this is a symmetric key, so the keystore will not let it encrypt
     * without authentication either, and no seal can be made behind the user's
     * back. This is the moment the finger is assigned to the box — null when the
     * user backs out of the prompt, a fault throws so Rust can tell the two apart.
     */
    static String seal(String plain) {
        byte[] raw = Base64.decode(plain, Base64.NO_WRAP);
        try {
            Cipher cipher = Cipher.getInstance(CIPHER);
            cipher.init(Cipher.ENCRYPT_MODE, key());
            byte[] sealed = authenticate(cipher, raw,
                    "Set up fingerprint unlock",
                    "Confirm your fingerprint to keep this box's key");
            if (sealed == null) {
                return null;
            }
            byte[] iv = cipher.getIV();
            byte[] blob = new byte[1 + iv.length + sealed.length];
            blob[0] = (byte) iv.length;
            System.arraycopy(iv, 0, blob, 1, iv.length);
            System.arraycopy(sealed, 0, blob, 1 + iv.length, sealed.length);
            Arrays.fill(sealed, (byte) 0);
            return Base64.encodeToString(blob, Base64.NO_WRAP);
        } catch (Exception e) {
            throw new IllegalStateException("the keystore would not seal the key: " + e, e);
        } finally {
            Arrays.fill(raw, (byte) 0);
        }
    }

    /**
     * The box's derived key, released by a fingerprint. Null when the user backed
     * out of the prompt; a fault throws, so Rust can tell the two apart.
     */
    static String unseal(String blob) {
        byte[] raw = Base64.decode(blob, Base64.NO_WRAP);
        if (raw.length < 2) {
            return null;
        }
        int ivLength = raw[0] & 0xFF;
        if (ivLength == 0 || raw.length <= 1 + ivLength) {
            return null;
        }
        byte[] iv = Arrays.copyOfRange(raw, 1, 1 + ivLength);
        byte[] sealed = Arrays.copyOfRange(raw, 1 + ivLength, raw.length);
        Cipher cipher;
        try {
            cipher = Cipher.getInstance(CIPHER);
            cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(128, iv));
        } catch (Exception e) {
            throw new IllegalStateException("the keystore key is gone: " + e, e);
        }
        try {
            // The ciphertext goes through the prompt with the cipher: a GCM tag is
            // only checked by the operation that authenticates it.
            byte[] opened = authenticate(cipher, sealed,
                    "Unlock this box",
                    "Pandora keeps this box's key behind your fingerprint");
            return opened == null ? null : Base64.encodeToString(opened, Base64.NO_WRAP);
        } finally {
            Arrays.fill(sealed, (byte) 0);
        }
    }

    /**
     * Open the back camera and keep its newest frame, for as long as a scan is
     * up. Welcome to camera2, which is verbose because it is the framework: one
     * device, one reader, one session, and the frames come out as NV21 because
     * that is the one layout both sides can turn into pixels cheaply.
     *
     * <p>Blocks this thread while the permission is asked for and while the
     * camera comes up — the callers are workers, as they are for the picker.
     * Answers `WxH <degrees>` when it is up, or a sentence saying why not.
     */
    static String startCamera() {
        Activity current = activity;
        if (current == null) {
            return "no window to open a camera in";
        }
        if (current.checkSelfPermission(Manifest.permission.CAMERA)
                != PackageManager.PERMISSION_GRANTED) {
            CountDownLatch asked = new CountDownLatch(1);
            asking = asked;
            current.runOnUiThread(() -> ActivityCompat.requestPermissions(
                    current, new String[] {Manifest.permission.CAMERA}, REQUEST_CAMERA));
            await(asked);
            asking = null;
            if (current.checkSelfPermission(Manifest.permission.CAMERA)
                    != PackageManager.PERMISSION_GRANTED) {
                return "the camera was not allowed";
            }
        }
        stopCamera();
        CameraManager manager = (CameraManager) current.getSystemService(Context.CAMERA_SERVICE);
        try {
            String found = null;
            for (String each : manager.getCameraIdList()) {
                Integer facing = manager.getCameraCharacteristics(each)
                        .get(CameraCharacteristics.LENS_FACING);
                if (facing != null && facing == CameraCharacteristics.LENS_FACING_BACK) {
                    found = each;
                    break;
                }
            }
            if (found == null) {
                return "this device has no back camera";
            }
            // Final because the callbacks below close over it.
            final String chosen = found;
            CameraCharacteristics traits = manager.getCameraCharacteristics(chosen);
            Size size = frameToRead(traits);
            frameWidth = size.getWidth();
            frameHeight = size.getHeight();
            Integer sensor = traits.get(CameraCharacteristics.SENSOR_ORIENTATION);
            // What the frame needs to stand up: the sensor is landscape on every
            // phone, and the app is upright.
            frameTurn = sensor == null ? 0 : sensor;
            opening = new CountDownLatch(1);
            cameraError = null;
            frames = null;
            // Not named `camera`: that is the device, and the callbacks below
            // assign to it.
            Handler thread = cameraThread();
            try {
                manager.openCamera(chosen, new CameraDevice.StateCallback() {
                        @Override
                        public void onOpened(CameraDevice device) {
                            camera = device;
                            watching(device);
                        }

                        @Override
                        public void onDisconnected(CameraDevice device) {
                            device.close();
                        }

                        @Override
                        public void onError(CameraDevice device, int error) {
                            cameraError = "the camera could not be opened: " + error;
                            device.close();
                            opening.countDown();
                        }
                }, thread);
            } catch (Exception e) {
                cameraError = "the camera could not be opened: " + e;
                opening.countDown();
            }
            await(opening);
            opening = null;
            if (cameraError != null) {
                String why = cameraError;
                cameraError = null;
                stopCamera();
                return why;
            }
            return frameWidth + "x" + frameHeight + " " + frameTurn;
        } catch (Exception e) {
            stopCamera();
            return "the camera could not be opened: " + e;
        }
    }

    /**
     * The thread the camera works on. camera2 wants a looper: opening, the
     * session, and every frame come back there, so none of it lands on the
     * thread that draws.
     */
    private static Handler cameraThread() {
        Handler made = onWorker;
        if (made == null) {
            HandlerThread thread = new HandlerThread("pandora-camera");
            thread.start();
            made = new Handler(thread.getLooper());
            worker = thread;
            onWorker = made;
        }
        return made;
    }

    /** The newest frame as NV21, or null while none has arrived. */
    static byte[] cameraFrame() {
        return frames;
    }

    static void stopCamera() {
        frames = null;
        try {
            if (session != null) {
                session.close();
            }
        } catch (Exception ignored) {
            // Closing twice, or after the camera went away, is not a fault.
        }
        session = null;
        try {
            if (camera != null) {
                camera.close();
            }
        } catch (Exception ignored) {
            // Same.
        }
        camera = null;
        if (reader != null) {
            reader.close();
            reader = null;
        }
        if (worker != null) {
            worker.quitSafely();
            worker = null;
            onWorker = null;
        }
    }

    /** The permission prompt's answer, from the activity (UI thread). */
    static void asked(int requestCode, int[] granted) {
        CountDownLatch waiting = asking;
        if (requestCode != REQUEST_CAMERA || waiting == null) {
            return;
        }
        waiting.countDown();
    }

    /**
     * The frame to read a QR out of: the biggest one within 1080p. Not the
     * smallest: the sensor is many times that, and downscaling it in the HAL is
     * what turns the screen's own pixel grid into noise across every module. Hand
     * the reader the pixels and let the other side smooth them properly.
     */
    private static Size frameToRead(CameraCharacteristics traits) {
        Size[] all = traits.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                .getOutputSizes(ImageFormat.YUV_420_888);
        Size best = null;
        for (Size size : all) {
            long pixels = (long) size.getWidth() * size.getHeight();
            if (pixels > 1920L * 1080L) {
                continue;
            }
            if (best == null || pixels > (long) best.getWidth() * best.getHeight()) {
                best = size;
            }
        }
        return best != null ? best : all[0];
    }

    /** The reader and the session that keep the newest frame coming. */
    private static void watching(CameraDevice device) {
        try {
            Handler thread = cameraThread();
            reader = ImageReader.newInstance(frameWidth, frameHeight, ImageFormat.YUV_420_888, 2);
            reader.setOnImageAvailableListener(from -> {
                Image image = null;
                try {
                    image = from.acquireLatestImage();
                    if (image != null) {
                        frames = nv21(image);
                    }
                } catch (Exception e) {
                    cameraError = "the camera failed: " + e;
                } finally {
                    if (image != null) {
                        image.close();
                    }
                }
            }, thread);
            device.createCaptureSession(List.of(reader.getSurface()),
                    new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession configured) {
                            session = configured;
                            try {
                                CaptureRequest.Builder request =
                                        device.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
                                request.addTarget(reader.getSurface());
                                configured.setRepeatingRequest(request.build(), null, thread);
                            } catch (Exception e) {
                                cameraError = "the camera failed: " + e;
                            }
                            opening.countDown();
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession failed) {
                            cameraError = "the camera could not be set up";
                            opening.countDown();
                        }
                    }, thread);
        } catch (Exception e) {
            cameraError = "the camera failed: " + e;
            opening.countDown();
        }
    }

    /**
     * The frame as NV21 — every luma, then the chroma interleaved — which is the
     * layout the other side turns into pixels in one pass. The planes carry row
     * strides, so it is not a copy.
     */
    private static byte[] nv21(Image image) {
        int width = image.getWidth();
        int height = image.getHeight();
        byte[] out = new byte[width * height * 3 / 2];
        Image.Plane[] planes = image.getPlanes();
        Image.Plane luma = planes[0];
        ByteBuffer ys = luma.getBuffer();
        int yRow = luma.getRowStride();
        int yStep = luma.getPixelStride();
        for (int row = 0; row < height; row++) {
            for (int col = 0; col < width; col++) {
                out[row * width + col] = ys.get(row * yRow + col * yStep);
            }
        }
        Image.Plane u = planes[1];
        Image.Plane v = planes[2];
        ByteBuffer us = u.getBuffer();
        ByteBuffer vs = v.getBuffer();
        int uRow = u.getRowStride();
        int uStep = u.getPixelStride();
        int vRow = v.getRowStride();
        int vStep = v.getPixelStride();
        int at = width * height;
        for (int row = 0; row < height / 2; row++) {
            for (int col = 0; col < width / 2; col++) {
                out[at++] = vs.get(row * vRow + col * vStep);
                out[at++] = us.get(row * uRow + col * uStep);
            }
        }
        return out;
    }

    /**
     * Ask the picker for a file of `mime` — a box, or the picture a QR is in —
     * and copy it into the app's own directory: a `content://` URI is not a path
     * the vault could reopen tomorrow, so what the caller gets is that copy.
     * Blocks this thread; the prompt and the picker run on the UI thread, which
     * is why the callers are workers.
     */
    static String pick(String mime) {
        Activity current = activity;
        if (current == null) {
            return null;
        }
        CountDownLatch done = new CountDownLatch(1);
        picking = done;
        picked = null;
        failure = null;
        current.runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(mime == null || mime.isEmpty() ? "*/*" : mime);
            current.startActivityForResult(intent, REQUEST_PICK);
        });
        await(done);
        picking = null;
        String path = picked;
        picked = null;
        if (path == null && failure != null) {
            String why = failure;
            failure = null;
            throw new IllegalStateException(why);
        }
        return path;
    }

    /** The picker's answer, from the activity's `onActivityResult` (UI thread). */
    static void picked(int requestCode, int resultCode, Intent data) {
        CountDownLatch waiting = picking;
        if (requestCode != REQUEST_PICK || waiting == null) {
            return;
        }
        try {
            if (resultCode == Activity.RESULT_OK && data != null && data.getData() != null) {
                picked = importTo(data.getData());
            }
        } catch (Exception e) {
            failure = "the picked file could not be copied: " + e;
        }
        waiting.countDown();
    }

    static void copy(String text) {
        ClipboardManager clipboard = clipboard();
        if (clipboard == null) {
            return;
        }
        ClipData clip = ClipData.newPlainText("Pandora", text);
        // Keeps a copied password out of the clipboard preview and off the history
        // on 13+. Spelled out because the constant only exists from 33 on, and a
        // secret must not need a version guard to stay secret.
        PersistableBundle extras = new PersistableBundle();
        extras.putBoolean("android.content.extra.IS_SENSITIVE", true);
        clip.getDescription().setExtras(extras);
        clipboard.setPrimaryClip(clip);
    }

    static String paste() {
        ClipboardManager clipboard = clipboard();
        if (clipboard == null) {
            return null;
        }
        ClipData clip = clipboard.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) {
            return null;
        }
        CharSequence text = clip.getItemAt(0).coerceToText(activity);
        return text == null ? null : text.toString();
    }

    static void clear() {
        ClipboardManager clipboard = clipboard();
        if (clipboard != null) {
            clipboard.clearPrimaryClip();
        }
    }

    /**
     * Show a page in a custom tab: the browser the phone already has, so the
     * sign-in finds whatever session is there and stays inside the app instead of
     * handing the link to the GitHub app. Best effort — with no browser at all,
     * the code and the address stay on screen, which is still enough to sign in
     * by hand.
     */
    static void open(String url) {
        Activity current = activity;
        if (current == null) {
            return;
        }
        Uri page = Uri.parse(url);
        current.runOnUiThread(() -> {
            try {
                new CustomTabsIntent.Builder()
                        // The tab draws no address bar, so the title is what says
                        // whose page this is. A sign-in with no host in sight reads
                        // like a form the app painted itself.
                        .setShowTitle(true)
                        .build()
                        .launchUrl(current, page);
            } catch (Exception e) {
                // No custom-tab browser: a plain view is still a browser.
                try {
                    current.startActivity(new Intent(Intent.ACTION_VIEW, page));
                } catch (Exception ignored) {
                    // And with nothing to take it either, the page stays on screen.
                }
            }
        });
    }

    /**
     * Keep the GitHub token: the keystore holds the key, the app writes down only
     * the ciphertext. No finger is asked for — this direction is the silent one,
     * which is the whole point of remembering a password.
     */
    static void keep(String token) {
        Activity current = activity;
        if (current == null) {
            throw new IllegalStateException("the app has no activity");
        }
        try {
            Cipher cipher = Cipher.getInstance(CIPHER);
            cipher.init(Cipher.ENCRYPT_MODE, tokenKey());
            byte[] sealed = cipher.doFinal(token.getBytes(StandardCharsets.UTF_8));
            byte[] iv = cipher.getIV();
            byte[] blob = new byte[1 + iv.length + sealed.length];
            blob[0] = (byte) iv.length;
            System.arraycopy(iv, 0, blob, 1, iv.length);
            System.arraycopy(sealed, 0, blob, 1 + iv.length, sealed.length);
            Arrays.fill(sealed, (byte) 0);
            current.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit()
                    .putString(TOKEN, Base64.encodeToString(blob, Base64.NO_WRAP))
                    .apply();
            Log.i(TAG, "keep: sealed the github token (" + blob.length + " bytes)");
        } catch (Exception e) {
            throw new IllegalStateException("the keystore would not keep the token: " + e, e);
        }
    }

    /**
     * The token a previous run kept. Null when there is none, or its key is gone.
     *
     * A blob is only thrown away when nothing could ever read it again: a
     * malformed one, or one the key genuinely no longer opens. Everything else —
     * a keystore that is not up yet just after a boot, a driver that says no once
     * — leaves the ciphertext where it is and answers null, because dropping it
     * costs the session and this file is the only copy.
     */
    static String recall() {
        Activity current = activity;
        if (current == null) {
            Log.w(TAG, "recall: no activity");
            return null;
        }
        String blob = current.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getString(TOKEN, null);
        if (blob == null) {
            Log.i(TAG, "recall: nothing was kept");
            return null;
        }
        try {
            byte[] raw = Base64.decode(blob, Base64.NO_WRAP);
            if (raw.length < 2) {
                Log.w(TAG, "recall: the kept token is malformed (" + raw.length + " bytes)");
                drop();
                return null;
            }
            int ivLength = raw[0] & 0xFF;
            if (ivLength == 0 || raw.length <= 1 + ivLength) {
                Log.w(TAG, "recall: the kept token has no usable iv");
                drop();
                return null;
            }
            byte[] iv = Arrays.copyOfRange(raw, 1, 1 + ivLength);
            byte[] sealed = Arrays.copyOfRange(raw, 1 + ivLength, raw.length);
            Cipher cipher = Cipher.getInstance(CIPHER);
            cipher.init(Cipher.DECRYPT_MODE, tokenKey(), new GCMParameterSpec(128, iv));
            byte[] opened = cipher.doFinal(sealed);
            Arrays.fill(sealed, (byte) 0);
            return new String(opened, StandardCharsets.UTF_8);
        } catch (AEADBadTagException e) {
            // The key the token was sealed with is not the key this run has: it
            // was rotated or lost, and nothing will ever open this ciphertext.
            Log.w(TAG, "recall: the kept token no longer decrypts; dropping it", e);
            drop();
            return null;
        } catch (Exception e) {
            // Not an answer about the token. Keep the ciphertext and let the
            // next run try again — the user is asked to sign in either way, and
            // throwing it away here is what makes that permanent.
            Log.w(TAG, "recall: could not read the kept token: " + e, e);
            return null;
        }
    }

    /** The token the user asked to forget. */
    static void drop() {
        Activity current = activity;
        if (current != null) {
            current.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit()
                    .remove(TOKEN)
                    .apply();
        }
    }

    /**
     * Show the prompt on the UI thread and block until it answers. Only a live
     * finger may use the cipher's key — in either direction — so this call is what
     * performs the operation: `input` goes into the cipher the prompt authorized,
     * and what comes out is the result. Null when the user backed out; a fault
     * throws.
     */
    private static byte[] authenticate(final Cipher cipher, final byte[] input,
            final String title, final String subtitle) {
        final Activity current = activity;
        // GameActivity is an AppCompatActivity, which is the FragmentActivity the
        // androidx prompt asks for.
        if (!(current instanceof FragmentActivity)) {
            return null;
        }
        final FragmentActivity host = (FragmentActivity) current;
        final CountDownLatch done = new CountDownLatch(1);
        final byte[][] opened = new byte[1][];
        host.runOnUiThread(() -> {
            BiometricPrompt prompt = new BiometricPrompt(host, host.getMainExecutor(),
                    new BiometricPrompt.AuthenticationCallback() {
                        @Override
                        public void onAuthenticationSucceeded(
                                BiometricPrompt.AuthenticationResult result) {
                            try {
                                opened[0] =
                                        result.getCryptoObject().getCipher().doFinal(input);
                            } catch (Exception e) {
                                failure = "the keystore would not use the key: " + e;
                            } finally {
                                done.countDown();
                            }
                        }

                        @Override
                        public void onAuthenticationError(int code, CharSequence message) {
                            // Backing out is not a fault worth reporting.
                            if (code != BiometricPrompt.ERROR_USER_CANCELED
                                    && code != BiometricPrompt.ERROR_NEGATIVE_BUTTON
                                    && code != BiometricPrompt.ERROR_CANCELED) {
                                failure = message == null
                                        ? "the fingerprint reader refused" : message.toString();
                            }
                            done.countDown();
                        }

                        @Override
                        public void onAuthenticationFailed() {
                            // A finger that did not read: the prompt stays up.
                        }
                    });
            prompt.authenticate(new BiometricPrompt.PromptInfo.Builder()
                    .setTitle(title)
                    .setSubtitle(subtitle)
                    .setNegativeButtonText("Cancel")
                    // A real fingerprint, not the weak face check that a photo
                    // sometimes passes.
                    .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                    .build(), new BiometricPrompt.CryptoObject(cipher));
        });
        await(done);
        if (opened[0] == null && failure != null) {
            String why = failure;
            failure = null;
            throw new IllegalStateException(why);
        }
        return opened[0];
    }

    /**
     * The keystore key, made the first time it is needed. It demands user
     * authentication for every use, and a new fingerprint enrolled on the phone
     * invalidates it — which is why a re-enrolled finger falls back to the
     * password once, and the reason the seal is worth anything.
     */
    private static SecretKey key() throws Exception {
        return keystoreKey(KEY, true);
    }

    /**
     * The key the GitHub token is wrapped in: a keystore key too, but one that
     * asks for nothing. It has to open with no prompt, or remembering the
     * session would mean a fingerprint at every launch.
     */
    private static SecretKey tokenKey() throws Exception {
        return keystoreKey(TOKEN_KEY, false);
    }

    private static SecretKey keystoreKey(String alias, boolean authentication) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        Key existing = store.getKey(alias, null);
        if (existing instanceof SecretKey) {
            return (SecretKey) existing;
        }
        KeyGenParameterSpec.Builder spec = new KeyGenParameterSpec.Builder(alias,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE);
        if (authentication) {
            spec.setUserAuthenticationRequired(true)
                    .setUserAuthenticationValidityDurationSeconds(-1)
                    .setInvalidatedByBiometricEnrollment(true);
        }
        KeyGenerator generator =
                KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(spec.build());
        return generator.generateKey();
    }

    private static String importTo(Uri uri) throws IOException {
        Activity current = activity;
        if (current == null) {
            return null;
        }
        File dir = new File(current.getFilesDir(), "imported");
        if (!dir.isDirectory() && !dir.mkdirs()) {
            throw new IOException("cannot make " + dir);
        }
        File out = new File(dir, safeName(nameOf(uri)));
        try (InputStream in = current.getContentResolver().openInputStream(uri);
             OutputStream to = new FileOutputStream(out)) {
            if (in == null) {
                throw new IOException("cannot read the picked file");
            }
            byte[] buffer = new byte[8192];
            for (int read; (read = in.read(buffer)) > 0; ) {
                to.write(buffer, 0, read);
            }
        }
        return out.getAbsolutePath();
    }

    /** What the provider calls the file, when it says. */
    private static String nameOf(Uri uri) {
        try (Cursor cursor =
                     activity.getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int at = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (at >= 0) {
                    String name = cursor.getString(at);
                    if (name != null && !name.isEmpty()) {
                        return name;
                    }
                }
            }
        } catch (Exception e) {
            // A provider that will not answer gets the default name below.
        }
        return "imported";
    }

    /**
     * The reported name is from outside the app, so it is cut down to what cannot
     * walk out of the directory the copy lands in.
     */
    private static String safeName(String name) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            if (Character.isLetterOrDigit(c) || c == '.' || c == '_' || c == '-') {
                out.append(c);
            }
        }
        String cleaned = out.toString();
        if (cleaned.isEmpty() || cleaned.chars().allMatch(c -> c == '.')) {
            return "imported";
        }
        return cleaned;
    }

    private static ClipboardManager clipboard() {
        Activity current = activity;
        return current == null
                ? null
                : (ClipboardManager) current.getSystemService(Context.CLIPBOARD_SERVICE);
    }

    private static void await(CountDownLatch latch) {
        try {
            latch.await();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}
