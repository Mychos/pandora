package dev.pandora;

import android.app.PendingIntent;
import android.app.assist.AssistStructure;
import android.content.Intent;
import android.os.CancellationSignal;
import android.service.autofill.Dataset;
import android.service.autofill.FillCallback;
import android.service.autofill.FillContext;
import android.service.autofill.FillRequest;
import android.service.autofill.FillResponse;
import android.service.autofill.SaveCallback;
import android.service.autofill.SaveInfo;
import android.service.autofill.SaveRequest;
import android.text.InputType;
import android.util.Log;
import android.view.View;
import android.view.autofill.AutofillId;
import android.view.autofill.AutofillValue;
import android.widget.RemoteViews;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.List;
import java.util.Locale;

/**
 * Fills logins from the box the Pandora app has open.
 *
 * <p>This side only ever knows <em>where</em> a page keeps its fields; the Rust
 * half ({@link Autofill}) knows what goes in them, and never hands over a
 * password until the user picks an entry. A process can start on a fill request,
 * so {@link #onCreate()} is the first thing that runs and hands the context over.
 */
public class AutofillService extends android.service.autofill.AutofillService {

    /** The tag `adb logcat -s pandora` filters on. */
    private static final String TAG = "pandora";

    // shortcut: the one user-visible string this side owns lives here, so a
    // Spanish phone reads it in English; move it to res/values-{en,es} when the
    // panel needs a second one.
    private static final String OPEN = "Open Pandora";

    /** Where a page keeps the fields a login fills, and what page it is. */
    private static final class Fields {
        AutofillId username;
        AutofillId password;
        AutofillId otp;
        // The WebView's domain; null in a native app, which no stored URL claims.
        String page;
        // What the fields hold, read on a save request; null while filling.
        String usernameText;
        String passwordText;
    }

    @Override
    public void onCreate() {
        super.onCreate();
        Autofill.nativeInit(this);
    }

    @Override
    public void onFillRequest(FillRequest request, CancellationSignal cancel, FillCallback callback) {
        Fields fields = scan(request.getFillContexts());
        if (fields == null) {
            Log.i(TAG, "fill request: no login fields");
            callback.onSuccess(null);
            return;
        }
        if (!"ready".equals(Autofill.nativeState())) {
            // A fill request can start this process, and no box is open in it: the
            // only useful offer is the way to open one, which is what somebody who
            // tapped a field needs anyway.
            Log.i(TAG, "fill request: no box open");
            callback.onSuccess(open(fields));
            return;
        }
        // A box is open: logins for the page, and codes for a field that asks for one.
        FillResponse.Builder response = new FillResponse.Builder();
        int built = fields.password == null ? 0 : panel(fields, response);
        if (fields.otp != null) {
            built += codes(fields, response);
        }
        Log.i(TAG, "fill request: " + built + " offered");
        callback.onSuccess(built == 0 ? null : response.build());
    }

    /** One dataset per seed, with the code it holds now. */
    private int codes(Fields fields, FillResponse.Builder response) {
        String json = Autofill.nativeCodes();
        if (json == null) {
            return 0;
        }
        int built = 0;
        try {
            JSONArray seeds = new JSONArray(json);
            for (int i = 0; i < seeds.length(); i++) {
                Dataset dataset = code(fields, seeds.getJSONObject(i));
                if (dataset != null) {
                    response.addDataset(dataset);
                    built += 1;
                }
            }
        } catch (JSONException e) {
            Log.w(TAG, "autofill: unreadable seeds: " + e.getMessage());
        }
        return built;
    }

    /**
     * One row per seed: its name, the code it derives right now and the seconds
     * left on it. The seed never leaves the Rust side, and the code is the only
     * thing a page gets.
     */
    // The `RemoteViews` dataset is deprecated against this SDK, and it is the only
    // constructor an app can build: `Presentations` has no public one.
    @SuppressWarnings("deprecation")
    private Dataset code(Fields fields, JSONObject seed) throws JSONException {
        String id = seed.getString("id");
        String json = Autofill.nativeCode(id);
        if (json == null) {
            return null;
        }
        JSONObject derived = new JSONObject(json);
        String text = derived.getString("code");
        // The seconds left are shown, not stored: a code does not survive the panel.
        String until = text + " · " + derived.optLong("remaining") + "s";
        return new Dataset.Builder(row(seed.getString("name"), until, ""))
                .setId(id)
                .setValue(fields.otp, AutofillValue.forText(text))
                .build();
    }

    /** One dataset per entry the box says claims this page, and how many landed. */
    private int panel(Fields fields, FillResponse.Builder response) {
        if (fields.page == null) {
            return 0;
        }
        String json = Autofill.nativeQuery(fields.page);
        if (json == null) {
            return 0;
        }
        int built = 0;
        try {
            JSONArray entries = new JSONArray(json);
            for (int i = 0; i < entries.length(); i++) {
                Dataset dataset = offer(fields, entries.getJSONObject(i));
                if (dataset != null) {
                    response.addDataset(dataset);
                    built += 1;
                }
            }
        } catch (JSONException e) {
            // The Rust side writes this JSON itself; a parse failure is a bug there,
            // not a reason to fail the fill request.
            Log.w(TAG, "autofill: unreadable candidates: " + e.getMessage());
        }
        // What the framework may offer to save once the user submits this form.
        if (fields.password != null) {
            response.setSaveInfo(save(fields));
        }
        return built;
    }

    /** The fields a save would take, and the kinds the framework labels it with. */
    private static SaveInfo save(Fields fields) {
        if (fields.username == null) {
            return new SaveInfo.Builder(
                            SaveInfo.SAVE_DATA_TYPE_PASSWORD, new AutofillId[] {fields.password})
                    .build();
        }
        return new SaveInfo.Builder(
                        SaveInfo.SAVE_DATA_TYPE_USERNAME | SaveInfo.SAVE_DATA_TYPE_PASSWORD,
                        new AutofillId[] {fields.username, fields.password})
                .build();
    }

    /**
     * A login the user just typed, handed to the box. Whether it is new is the
     * app's call: an entry with the same name, username and password is already
     * there, which is the rule an import uses too.
     */
    @Override
    public void onSaveRequest(SaveRequest request, SaveCallback callback) {
        Fields fields = scan(request.getFillContexts());
        // No page, no entry: a URL is how a login is found again, and a native app
        // has none to key it to.
        if (fields == null || fields.page == null || fields.passwordText == null) {
            Log.i(TAG, "save request: nothing usable");
            callback.onSuccess();
            return;
        }
        String user = fields.usernameText == null ? "" : fields.usernameText;
        String url = "https://" + fields.page;
        String queued = Autofill.nativeSave(fields.page, user, fields.passwordText, url);
        // This side keeps nothing once the app has it.
        fields.passwordText = null;
        Log.i(TAG, "save request: " + (queued == null ? "no box open" : "queued"));
        callback.onSuccess();
    }

    /**
     * One row the user may pick. The view gets the name, the username and the URL
     * that claimed the page; the password only ever goes into a field value, so
     * nothing it is painted with could show it.
     */
    // The `RemoteViews` dataset is deprecated against this SDK, and it is the only
    // constructor an app can build: `Presentations` has no public one. minSdk 29
    // has nothing else, so the deprecation is the platform's future, not a mistake.
    @SuppressWarnings("deprecation")
    private Dataset offer(Fields fields, JSONObject entry) throws JSONException {
        String id = entry.getString("id");
        String uri = entry.optString("uri");
        RemoteViews row = row(entry.getString("name"), entry.optString("username"), uri);
        String fill = Autofill.nativeFill(id);
        if (fill == null) {
            // The box left the screen between the query and the pick.
            return null;
        }
        JSONObject secret = new JSONObject(fill);
        Dataset.Builder dataset = new Dataset.Builder(row).setId(id);
        if (fields.username != null) {
            dataset.setValue(fields.username, AutofillValue.forText(secret.optString("username")));
        }
        dataset.setValue(fields.password, AutofillValue.forText(secret.getString("password")));
        return dataset.build();
    }

    /** A panel row; an empty or missing line is left out instead of drawn blank. */
    private RemoteViews row(String name, String username, String uri) {
        RemoteViews row = new RemoteViews(getPackageName(), R.layout.autofill_row);
        row.setTextViewText(R.id.autofill_name, name);
        row.setTextViewText(R.id.autofill_user, username);
        row.setTextViewText(R.id.autofill_uri, uri);
        row.setViewVisibility(R.id.autofill_user, username.isEmpty() ? View.GONE : View.VISIBLE);
        row.setViewVisibility(R.id.autofill_uri, uri.isEmpty() ? View.GONE : View.VISIBLE);
        return row;
    }

    /** One row that opens the app and comes back to the field the user tapped. */
    // The `RemoteViews` dataset is deprecated against this SDK, and it is the only
    // constructor an app can build: `Presentations` has no public one. minSdk 29
    // has nothing else, so the deprecation is the platform's future, not a mistake.
    @SuppressWarnings("deprecation")
    private FillResponse open(Fields fields) {
        Intent intent = new Intent(this, MainActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        PendingIntent pending = PendingIntent.getActivity(
                this,
                0,
                intent,
                PendingIntent.FLAG_CANCEL_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        RemoteViews row = row(OPEN, "", "");
        Dataset.Builder dataset = new Dataset.Builder(row)
                .setId("open")
                .setAuthentication(pending.getIntentSender());
        // A dataset with no value at all is refused by `build()` ("at least one of
        // field or datatype must be set"), so every field it was offered for gets an
        // empty one. They are applied only if the auth intent answers OK, and this
        // one opens the app and answers cancelled, so nothing is ever written.
        for (AutofillId id : new AutofillId[] {fields.username, fields.password, fields.otp}) {
            if (id != null) {
                dataset.setValue(id, AutofillValue.forText(""));
            }
        }
        return new FillResponse.Builder().addDataset(dataset.build()).build();
    }

    /** The fields of the last window in front of the user, or null when there are none. */
    private static Fields scan(List<FillContext> contexts) {
        AssistStructure structure = null;
        if (!contexts.isEmpty()) {
            structure = contexts.get(contexts.size() - 1).getStructure();
        }
        if (structure == null) {
            return null;
        }
        Fields fields = new Fields();
        for (int window = 0; window < structure.getWindowNodeCount(); window++) {
            walk(structure.getWindowNodeAt(window).getRootViewNode(), fields);
        }
        // A password field makes a page a login; a one-time code field makes it a
        // second step. A lone username field is a search box too often to offer
        // anything in.
        return fields.password == null && fields.otp == null ? null : fields;
    }

    private static void walk(AssistStructure.ViewNode node, Fields fields) {
        if (node == null) {
            return;
        }
        if (fields.page == null && node.getWebDomain() != null) {
            fields.page = node.getWebDomain();
        }
        if (node.getVisibility() == View.VISIBLE) {
            classify(node, fields);
        }
        for (int child = 0; child < node.getChildCount(); child++) {
            walk(node.getChildAt(child), fields);
        }
    }

    /**
     * Simple on purpose: the input type and the hints the field carries, never a
     * guess from the page's design. A field already found keeps the first spot,
     * so a form with two passwords offers the one nearest the top.
     */
    private static void classify(AssistStructure.ViewNode node, Fields fields) {
        AutofillId id = node.getAutofillId();
        if (id == null) {
            return;
        }
        String hint = hint(node);
        if (isPassword(node.getInputType()) || hint.contains("password") || hint.contains("passwd")) {
            if (fields.password == null) {
                fields.password = id;
                fields.passwordText = text(node);
            }
            return;
        }
        if (fields.otp == null && isCode(node.getInputType(), hint)) {
            fields.otp = id;
            return;
        }
        if (fields.username == null
                && (hint.contains("user")
                        || hint.contains("email")
                        || hint.contains("login")
                        || hint.contains("account"))) {
            fields.username = id;
            fields.usernameText = text(node);
        }
    }

    /** What a field holds right now; null on a fill request, where it is empty. */
    private static String text(AssistStructure.ViewNode node) {
        CharSequence text = node.getText();
        return text == null ? null : text.toString();
    }

    /**
     * A field that wants a one-time code. "otp" and its aliases are enough on
     * their own; a bare "code" has to be a number field too, because postal codes
     * and promo boxes are called that as well.
     */
    private static boolean isCode(int inputType, String hint) {
        boolean numeric = (inputType & InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_NUMBER;
        return hint.contains("otp")
                || hint.contains("one-time")
                || hint.contains("onetime")
                || hint.contains("totp")
                || hint.contains("2fa")
                || hint.contains("token")
                || (numeric && hint.contains("code"));
    }

    private static boolean isPassword(int inputType) {
        int variation = inputType & InputType.TYPE_MASK_VARIATION;
        return variation == InputType.TYPE_TEXT_VARIATION_PASSWORD
                || variation == InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                || variation == InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
                || variation == InputType.TYPE_NUMBER_VARIATION_PASSWORD;
    }

    /** Everything a field says about itself, in one lowercase string to look in. */
    private static String hint(AssistStructure.ViewNode node) {
        StringBuilder text = new StringBuilder();
        CharSequence[] hints = node.getAutofillHints();
        if (hints != null) {
            for (CharSequence hint : hints) {
                text.append(hint).append(' ');
            }
        }
        if (node.getIdEntry() != null) {
            text.append(node.getIdEntry()).append(' ');
        }
        if (node.getHint() != null) {
            text.append(node.getHint());
        }
        return text.toString().toLowerCase(Locale.ROOT);
    }
}
