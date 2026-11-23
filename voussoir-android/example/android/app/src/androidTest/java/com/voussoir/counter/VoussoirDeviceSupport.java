package com.voussoir.counter;

import static org.junit.Assert.fail;

import android.Manifest;
import android.accessibilityservice.AccessibilityService;
import android.app.Notification;
import android.app.NotificationManager;
import android.app.UiAutomation;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Rect;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.service.notification.StatusBarNotification;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.accessibility.AccessibilityNodeInfo;
import android.webkit.WebView;

import androidx.test.core.app.ActivityScenario;
import androidx.test.platform.app.InstrumentationRegistry;

import org.json.JSONTokener;

import java.io.ByteArrayOutputStream;
import java.io.FileInputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * What the device test needs that JUnit does not provide.
 *
 * <p>One helper class rather than a base class per test, because there is one
 * device test: see {@code VoussoirDeviceTest} for why that cannot be more.
 *
 * <p>UiAutomator is deliberately not a dependency. Everything needed is already
 * reachable through {@link UiAutomation}, which instrumentation owns: the active
 * window, its node tree, node actions, global actions, runtime permissions, and
 * single shell programs. Adding a test-only artifact to every scaffolded app to
 * obtain less than this would be a poor trade.
 */
final class VoussoirDeviceSupport {
    /** How long anything asynchronous is given: page loads, pickers, notifications. */
    static final long TIMEOUT_SECONDS = 40;

    /** Labels the system picker supplies, which are not this app's strings. */
    static final String SHOW_ROOTS = "Show roots";

    static final String DOWNLOADS = "Downloads";

    /** The id {@code VoussoirTray} always posts under. */
    static final int TRAY_ID = 1;

    /** A condition that holds once the system has caught up. */
    interface Condition {
        boolean holds();
    }

    /**
     * The one activity the whole run shares, launched at most once per process.
     *
     * <p>The shell starts its Rust event loop from {@code ProcessLifecycleOwner},
     * which runs once per process, so a second {@code ActivityScenario.launch}
     * produces an activity whose WebView has no page behind it. The page then never
     * leaves loading, and everything after it fails for a reason unrelated to what
     * it asserts. One activity per run is the arrangement the shell actually
     * supports, which is also why the device test is one method.
     */
    private static ActivityScenario<MainActivity> app;

    /** The shared activity, launched and waited for on first use. */
    static synchronized ActivityScenario<MainActivity> app() {
        if (app == null) {
            // Granted before the launch, not during: the tray posts on startup, so
            // the app raises the notification prompt over itself, and a prompt
            // leaves the activity paused with its WebView unable to run a script.
            grant(Manifest.permission.POST_NOTIFICATIONS);
            app = ActivityScenario.launch(MainActivity.class);
            awaitPage(app);
        }
        return app;
    }

    private VoussoirDeviceSupport() {}

    static UiAutomation ui() {
        return InstrumentationRegistry.getInstrumentation().getUiAutomation();
    }

    /** The package under test, which is also the app whose notifications land here. */
    static String packageName() {
        return InstrumentationRegistry.getInstrumentation().getTargetContext().getPackageName();
    }

    /**
     * Grant a runtime permission, as the identity instrumentation runs under.
     *
     * <p>Answering the OS prompt instead would not do: a prompt is a window in
     * front of the app, so the activity under test stops being resumed for as long
     * as it is up. Granting first removes that variable rather than racing it.
     *
     * <p>Silently does nothing for a permission the manifest does not declare,
     * because the platform throws for those and a test about notifications should
     * not fail in an app that has no tray.
     */
    static void grant(String permission) {
        if (!declares(permission)) {
            return;
        }
        ui().grantRuntimePermission(packageName(), permission);
    }

    /** Whether this app currently holds {@code permission}. */
    static boolean holds(String permission) {
        return InstrumentationRegistry.getInstrumentation()
                        .getTargetContext()
                        .getPackageManager()
                        .checkPermission(permission, packageName())
                == PackageManager.PERMISSION_GRANTED;
    }

    /** Whether this app's manifest requests {@code permission} at all. */
    static boolean declares(String permission) {
        try {
            PackageInfo info = InstrumentationRegistry.getInstrumentation()
                    .getTargetContext()
                    .getPackageManager()
                    .getPackageInfo(packageName(), PackageManager.GET_PERMISSIONS);
            if (info.requestedPermissions == null) {
                return false;
            }
            for (String requested : info.requestedPermissions) {
                if (permission.equals(requested)) {
                    return true;
                }
            }
            return false;
        } catch (PackageManager.NameNotFoundException e) {
            throw new AssertionError("the package under test is not installed", e);
        }
    }

    /**
     * Run one shell program and return its stdout.
     *
     * <p>{@code executeShellCommand} runs the program directly rather than through
     * {@code /system/bin/sh}, so redirection, globs and pipelines are not
     * interpreted: {@code echo hi > file} becomes an {@code echo} that prints
     * "hi > file". Anything beyond running one program and reading its output has
     * to be done in Java.
     */
    static String shell(String command) {
        try (ParcelFileDescriptor descriptor = ui().executeShellCommand(command);
                FileInputStream output = new FileInputStream(descriptor.getFileDescriptor())) {
            ByteArrayOutputStream buffer = new ByteArrayOutputStream();
            byte[] chunk = new byte[4096];
            int read;
            while ((read = output.read(chunk)) > 0) {
                buffer.write(chunk, 0, read);
            }
            return buffer.toString("UTF-8");
        } catch (IOException e) {
            throw new AssertionError("the shell could not run " + command, e);
        }
    }

    /**
     * Put a file with known content in Downloads, the way a real download lands.
     *
     * <p>Through {@code MediaStore} rather than a path, because that is the only
     * identity allowed to write there: an app cannot write {@code /sdcard/Download}
     * directly under scoped storage, and the shell cannot read this app's private
     * cache to stage a copy out of it. MediaStore needs no permission on API 29 and
     * up, and a row inserted this way is exactly what the picker's Downloads
     * provider lists, so the fixture is also the realistic one.
     *
     * <p>{@code relativePath} is relative to the volume root, so {@code "Download"}
     * puts the file in Downloads and {@code "Download/sub"} creates {@code sub} as
     * well, which is how the folder case gets a directory it can select.
     */
    static void seedDownload(final String name, final String relativePath, final String contents) {
        final MainActivity[] activity = new MainActivity[1];
        app().onActivity(current -> activity[0] = current);

        android.content.ContentValues values = new android.content.ContentValues();
        values.put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, name);
        values.put(android.provider.MediaStore.MediaColumns.MIME_TYPE, "text/plain");
        values.put(android.provider.MediaStore.MediaColumns.RELATIVE_PATH, relativePath);

        android.net.Uri target = activity[0]
                .getContentResolver()
                .insert(android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
        if (target == null) {
            throw new AssertionError("MediaStore refused to create " + relativePath + "/" + name);
        }
        try (java.io.OutputStream output =
                activity[0].getContentResolver().openOutputStream(target)) {
            if (output == null) {
                throw new AssertionError("MediaStore created " + target + " but would not open it");
            }
            output.write(contents.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        } catch (IOException e) {
            throw new AssertionError("could not write the fixture at " + target, e);
        }
    }

    /**
     * Whether this device currently has a working internet connection.
     *
     * <p>Asked with a single ping rather than {@code ConnectivityManager}: the
     * latter needs {@code ACCESS_NETWORK_STATE}, and a test is not a good reason to
     * add a permission to every scaffolded app. A failed ping is the same question
     * answered without touching the manifest.
     */
    static boolean online() {
        String output = shell("ping -c 1 -W 2 example.com");
        return output.contains("1 received") || output.contains("1 packets received");
    }

    /**
     * Read a file as this app, not as the shell.
     *
     * <p>The shell cannot descend into another app's private directory, so
     * verifying a cache copy from there always reads nothing. The app process is the
     * only identity that can prove the path it was handed names a file it can open,
     * which is the property the picker's copy branch exists to provide.
     */
    static String readFile(String path) {
        final String[] contents = new String[1];
        app().onActivity(activity -> {
            java.io.File file = new java.io.File(path);
            byte[] bytes = new byte[(int) file.length()];
            try (FileInputStream input = new FileInputStream(file)) {
                int read = 0;
                while (read < bytes.length) {
                    int justRead = input.read(bytes, read, bytes.length - read);
                    if (justRead < 0) {
                        break;
                    }
                    read += justRead;
                }
                contents[0] = new String(bytes, java.nio.charset.StandardCharsets.UTF_8).trim();
            } catch (IOException e) {
                contents[0] = null;
            }
        });
        return contents[0];
    }

    /**
     * Fetch a URL the way Rust does: on a worker thread, through the activity.
     *
     * <p>{@code HttpURLConnection} throws {@code NetworkOnMainThreadException} on
     * the UI thread, which is why the shell calls this from the updater's worker. A
     * test calling it from {@code onActivity} would measure the wrong thing and fail
     * every time.
     */
    static String httpGet(String url) {
        final MainActivity[] activity = new MainActivity[1];
        app().onActivity(current -> activity[0] = current);
        final String[] body = new String[1];
        Thread worker = new Thread(() -> body[0] = activity[0].voussoirHttpGet(url), "voussoir-http");
        worker.start();
        try {
            worker.join(TimeUnit.SECONDS.toMillis(TIMEOUT_SECONDS));
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError("interrupted", e);
        }
        return body[0];
    }

    /** Poll {@code condition} until it holds, failing with {@code what} if it never does. */
    static void waitFor(String what, Condition condition) {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(TIMEOUT_SECONDS);
        while (System.nanoTime() < deadline) {
            if (condition.holds()) {
                return;
            }
            sleep(100);
        }
        fail("timed out waiting for " + what + "; active window labels: " + activeWindowLabels());
    }

    static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError("interrupted", e);
        }
    }

    /**
     * Wait until the page can run a script, which is when commands may be sent.
     *
     * <p>Deliberately does not press Back on anything: Back is delivered to the
     * window in front, and when that is this app it finishes it. An earlier version
     * dismissed the startup notification prompt that way and destroyed the activity
     * the whole run shares.
     */
    static void awaitPage(ActivityScenario<MainActivity> scenario) {
        waitFor("the page to load", () -> "complete".equals(value(scenario, "document.readyState")));
    }

    /**
     * Wait until the activity reports one orientation.
     *
     * <p>A rotation is asynchronous: {@code setRequestedOrientation} returns long
     * before the configuration changes, so waiting on the configuration is the only
     * honest way to know it settled.
     */
    static void awaitOrientation(ActivityScenario<MainActivity> scenario, int wanted) {
        waitFor(
                wanted == Configuration.ORIENTATION_LANDSCAPE ? "landscape" : "portrait",
                () -> {
                    boolean[] reached = new boolean[1];
                    scenario.onActivity(activity -> reached[0] =
                            activity.getResources().getConfiguration().orientation == wanted);
                    return reached[0];
                });
    }

    /** Wait until the activity reports itself resumed. */
    static void awaitResumed(ActivityScenario<MainActivity> scenario) {
        waitFor("the activity to be resumed", () -> {
            boolean[] resumed = new boolean[1];
            scenario.onActivity(activity -> resumed[0] = activity.getLifecycle().getCurrentState()
                    == androidx.lifecycle.Lifecycle.State.RESUMED);
            return resumed[0];
        });
    }

    /** Wait until {@code expected} owns the window in front, i.e. it has won focus. */
    static void awaitPackage(String expected) {
        waitFor("the " + expected + " window", () -> expected.equals(activePackage()));
    }

    /**
     * Wait until something other than this app is in front, and return its package.
     *
     * <p>Used for the picker instead of resolving the intent, because Android 11
     * hides other packages from {@code queryIntentActivities} unless the app
     * declares a {@code <queries>} entry, and adding one to every scaffolded app so
     * a test can name the picker would be the wrong trade. The system starts the
     * picker either way, so the window that appears is the answer.
     */
    static String awaitForeignWindow(String what) {
        final String[] found = new String[1];
        waitFor(what, () -> {
            String active = activePackage();
            if (active == null || active.equals(packageName())) {
                return false;
            }
            found[0] = active;
            return true;
        });
        return found[0];
    }

    /** The package owning the window in front, or null when nothing has focus. */
    static String activePackage() {
        AccessibilityNodeInfo root = ui().getRootInActiveWindow();
        if (root == null || root.getPackageName() == null) {
            return null;
        }
        return root.getPackageName().toString();
    }

    /**
     * Dismiss an OS prompt sitting in front of this app, and wait for the app back.
     *
     * <p>Only the permission controller is dismissed. Pressing Back on anything else
     * would be delivered to whatever is in front, which finishes this app when the
     * launcher or a picker has already gone, and an activity is only {@code RESUMED}
     * while it holds focus.
     */
    static void dismissForeignWindows() {
        for (int attempt = 0; attempt < 4; attempt++) {
            String active = activePackage();
            if (active == null || active.equals(packageName())) {
                awaitPackage(packageName());
                return;
            }
            if (!active.contains("permissioncontroller")) {
                break;
            }
            back();
            sleep(500);
        }
        awaitPackage(packageName());
    }

    /**
     * The first node in the active window labelled {@code label}.
     *
     * <p>Matched case-insensitively because a Material button renders its text in
     * capitals while the string it was given is not, and preferring a node that can
     * be activated because the same label appears more than once in DocumentsUI and
     * the first in tree order is often a toolbar title that does nothing.
     */
    static AccessibilityNodeInfo find(String label) {
        AccessibilityNodeInfo root = ui().getRootInActiveWindow();
        if (root == null) {
            return null;
        }
        List<AccessibilityNodeInfo> matches = new ArrayList<>();
        collect(root, label, 0, matches);
        for (AccessibilityNodeInfo match : matches) {
            if (activatable(match)) {
                return match;
            }
        }
        return matches.isEmpty() ? null : matches.get(0);
    }

    /** Whether a node, or one of its ancestors, is clickable. */
    private static boolean activatable(AccessibilityNodeInfo node) {
        AccessibilityNodeInfo current = node;
        for (int up = 0; current != null && up < 6; up++) {
            if (current.isClickable()) {
                return true;
            }
            current = current.getParent();
        }
        return false;
    }

    private static void collect(
            AccessibilityNodeInfo node, String label, int depth, List<AccessibilityNodeInfo> out) {
        if (node == null || depth > 40) {
            return;
        }
        CharSequence text = node.getText();
        CharSequence description = node.getContentDescription();
        if ((text != null && text.toString().equalsIgnoreCase(label))
                || (description != null && description.toString().equalsIgnoreCase(label))) {
            out.add(node);
        }
        for (int i = 0; i < node.getChildCount(); i++) {
            collect(node.getChild(i), label, depth + 1, out);
        }
    }

    /** Every label the active window exposes, so a timeout says what IS there. */
    private static String activeWindowLabels() {
        AccessibilityNodeInfo root = ui().getRootInActiveWindow();
        if (root == null) {
            return "<no active window>";
        }
        List<String> labels = new ArrayList<>();
        collectLabels(root, labels, 0);
        return labels.toString();
    }

    private static void collectLabels(AccessibilityNodeInfo node, List<String> out, int depth) {
        if (node == null || depth > 40) {
            return;
        }
        CharSequence text = node.getText();
        CharSequence description = node.getContentDescription();
        if (text != null && text.length() > 0) {
            out.add(text.toString());
        } else if (description != null && description.length() > 0) {
            out.add(description.toString());
        }
        for (int i = 0; i < node.getChildCount(); i++) {
            collectLabels(node.getChild(i), out, depth + 1);
        }
    }

    /** Wait until no node in the active window is labelled {@code label}. */
    static void waitUntilGone(String label) {
        waitFor(
                "the " + label + " panel to close",
                () -> {
                    AccessibilityNodeInfo root = ui().getRootInActiveWindow();
                    if (root == null) {
                        return true;
                    }
                    List<AccessibilityNodeInfo> matches = new ArrayList<>();
                    collect(root, label, 0, matches);
                    return matches.isEmpty();
                });
    }

    /** Click the first node labelled {@code label}, waiting for it to appear. */
    static void click(String label) {
        waitFor(label, () -> find(label) != null);
        // Finding the node is not the same as being able to click it. A row the
        // provider inserted a moment ago can be laid out but not yet actionable,
        // and the picker's own list rebuilds while it loads, so one attempt
        // fails intermittently on a busy emulator. Retry until the normal
        // timeout instead of failing on the first refusal.
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(TIMEOUT_SECONDS);
        while (System.nanoTime() < deadline) {
            AccessibilityNodeInfo node = find(label);
            if (node != null && click(node)) {
                return;
            }
            sleep(200);
        }
        fail("could not click " + label);
    }

    /**
     * Click a node, by accessibility action when it is clickable and by injecting a
     * tap at its centre otherwise.
     *
     * <p>The second path is not a workaround for a bug: the picker's file rows put
     * the label on a {@code TextView} inside a clickable container, so the labelled
     * node refuses {@code ACTION_CLICK} while a real tap on it works. Injecting the
     * tap is what a user does, needs no test-only dependency, and needs no knowledge
     * of the container's id.
     */
    static boolean click(AccessibilityNodeInfo node) {
        AccessibilityNodeInfo clickable = node;
        for (int up = 0; clickable != null && up < 6; up++) {
            if (clickable.isClickable()
                    && clickable.performAction(AccessibilityNodeInfo.ACTION_CLICK)) {
                return true;
            }
            clickable = clickable.getParent();
        }
        return tap(node);
    }

    /** Tap the centre of a node's bounds, the way a finger would. */
    static boolean tap(AccessibilityNodeInfo node) {
        Rect bounds = new Rect();
        node.getBoundsInScreen(bounds);
        if (bounds.width() <= 0 || bounds.height() <= 0) {
            return false;
        }
        long now = SystemClock.uptimeMillis();
        float x = bounds.exactCenterX();
        float y = bounds.exactCenterY();
        MotionEvent down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, x, y, 0);
        MotionEvent up = MotionEvent.obtain(now, now + 60, MotionEvent.ACTION_UP, x, y, 0);
        ui().injectInputEvent(down, true);
        ui().injectInputEvent(up, true);
        down.recycle();
        up.recycle();
        return true;
    }

    /**
     * Click a row in the picker's file list, scrolling to it if needed.
     *
     * <p>The list holds whatever the device already had, so a name that is not on
     * screen yet is expected rather than a failure. Scrolling is what a user does to
     * find it, and gives up only once the list stops moving.
     */
    static void clickRow(String label) {
        // Look first, then scroll, then look again: a list that is still filling in
        // reports nothing scrollable yet, and that is a reason to wait rather than a
        // reason to give up. Both real failures this had were that race.
        String lastSeen = null;
        for (int attempt = 0; attempt < 20; attempt++) {
            AccessibilityNodeInfo row = find(label);
            if (row != null) {
                if (!click(row)) {
                    fail("the " + label + " row would not open");
                }
                return;
            }
            String first = firstVisibleLabel();
            if (scrollForward()) {
                sleep(400);
                // A scroll that changes nothing means the list has reached its end,
                // which is the only condition worth stopping for.
                if (first != null && first.equals(lastSeen)) {
                    break;
                }
            }
            lastSeen = first;
            sleep(400);
        }
        fail("the picker never showed " + label + "; the list ends at " + lastSeen);
    }

    /** The first labelled row on screen, used to tell "at the end" from "still loading". */
    private static String firstVisibleLabel() {
        AccessibilityNodeInfo list = findScrollable(ui().getRootInActiveWindow(), 0);
        if (list == null) {
            return null;
        }
        for (int i = 0; i < list.getChildCount(); i++) {
            CharSequence text = list.getChild(i).getText();
            if (text != null && text.length() > 0) {
                return text.toString();
            }
        }
        return null;
    }

    /** Scroll the active window's list forward; false when nothing can scroll. */
    static boolean scrollForward() {
        AccessibilityNodeInfo scrollable = findScrollable(ui().getRootInActiveWindow(), 0);
        return scrollable != null
                && scrollable.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
    }

    private static AccessibilityNodeInfo findScrollable(AccessibilityNodeInfo node, int depth) {
        if (node == null || depth > 40) {
            return null;
        }
        if (node.isScrollable()) {
            return node;
        }
        for (int i = 0; i < node.getChildCount(); i++) {
            AccessibilityNodeInfo found = findScrollable(node.getChild(i), depth + 1);
            if (found != null) {
                return found;
            }
        }
        return null;
    }

    /**
     * Whether a picker has handed control back to this app.
     *
     * <p>The honest test of a picker click is that the picker went away. A tap can
     * land on a row's preview affordance instead of the row itself, which leaves the
     * picker open with nothing chosen, and no assertion about the reply can tell
     * that apart from a slow picker.
     */
    static boolean pickerClosed() {
        String active = activePackage();
        return active == null || active.equals(packageName());
    }

    /** Wait a few seconds for the picker to close, returning whether it did. */
    static boolean awaitPickerClosed() {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(6);
        while (System.nanoTime() < deadline) {
            if (pickerClosed()) {
                return true;
            }
            sleep(250);
        }
        return pickerClosed();
    }

    /** Press the system back button, the way a user dismisses a picker. */
    static void back() {
        ui().performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK);
    }

    /** Start {@code command} in the page, leaving its reply in {@code window.__voussoirProbe}. */
    static void startInvoke(
            ActivityScenario<MainActivity> scenario, String command, String argsJson) {
        eval(
                scenario,
                "window.__voussoirProbe = 'pending';"
                        + "window.voussoir.invoke('" + command + "', " + argsJson + ").then("
                        + "function (r) { window.__voussoirProbe = JSON.stringify("
                        + "r === undefined ? null : r); },"
                        + "function (e) { window.__voussoirProbe = 'error: '"
                        + " + (e && e.message ? e.message : e); });");
    }

    /**
     * Wait for the reply {@link #startInvoke} asked for and return it as JSON text.
     *
     * <p>Three answers stay distinguishable on purpose: a value, the text
     * {@code null} for a resolved null (which is what a cancelled picker answers),
     * and {@code error: ...} for a rejected promise. A test that treated them alike
     * would read a crash as a cancel.
     */
    static String awaitProbe(ActivityScenario<MainActivity> scenario) {
        waitFor("the page to answer a command", () -> {
            String reply = probe(scenario);
            return reply != null && !reply.equals("pending");
        });
        return probe(scenario);
    }

    private static String probe(ActivityScenario<MainActivity> scenario) {
        String encoded = eval(scenario, "String(window.__voussoirProbe)");
        Object value;
        try {
            value = encoded == null ? null : new JSONTokener(encoded).nextValue();
        } catch (Exception e) {
            throw new AssertionError("the page probe was not JSON: " + encoded, e);
        }
        return value == null ? null : value.toString();
    }

    /** Evaluate {@code script} in the page and return the JSON-encoded result. */
    static String eval(ActivityScenario<MainActivity> scenario, String script) {
        CountDownLatch latch = new CountDownLatch(1);
        String[] reply = new String[1];
        scenario.onActivity(activity -> {
            WebView webView = webViewOf(activity);
            if (webView == null) {
                latch.countDown();
                return;
            }
            webView.evaluateJavascript(script, value -> {
                reply[0] = value;
                latch.countDown();
            });
        });
        try {
            if (!latch.await(TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
                throw new AssertionError("the page did not run: " + script);
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError("interrupted", e);
        }
        return reply[0];
    }

    /** Evaluate {@code script} and return its value as text, or null if undecodable. */
    static String value(ActivityScenario<MainActivity> scenario, String script) {
        try {
            Object decoded = new JSONTokener(eval(scenario, script)).nextValue();
            return decoded == null ? null : decoded.toString();
        } catch (Exception e) {
            return null;
        }
    }

    /**
     * The attached WebView, found by walking the content view.
     *
     * <p>Not held by the activity in a field a test can reach: wry keeps it private
     * and the app installs it with {@code setContentView}, so the view tree is the
     * only unambiguous place to look.
     */
    static WebView webViewOf(MainActivity activity) {
        View content = activity.findViewById(android.R.id.content);
        return content == null ? null : findWebView(content, 0);
    }

    private static WebView findWebView(View view, int depth) {
        if (view instanceof WebView) {
            return (WebView) view;
        }
        if (depth > 8 || !(view instanceof ViewGroup)) {
            return null;
        }
        ViewGroup group = (ViewGroup) view;
        for (int i = 0; i < group.getChildCount(); i++) {
            WebView found = findWebView(group.getChildAt(i), depth + 1);
            if (found != null) {
                return found;
            }
        }
        return null;
    }

    /** Whether the tray notification is currently posted. */
    static boolean trayPosted() {
        for (StatusBarNotification posted : notifications().getActiveNotifications()) {
            if (posted.getId() == TRAY_ID && packageName().equals(posted.getPackageName())) {
                return true;
            }
        }
        return false;
    }

    /** The posted tray notification's title, or null when it is not up. */
    static String trayTitle() {
        for (StatusBarNotification posted : notifications().getActiveNotifications()) {
            if (posted.getId() == TRAY_ID && packageName().equals(posted.getPackageName())) {
                CharSequence title =
                        posted.getNotification().extras.getCharSequence(Notification.EXTRA_TITLE);
                return title == null ? null : title.toString();
            }
        }
        return null;
    }

    /** How many action buttons the posted tray notification carries. */
    static int trayActions() {
        for (StatusBarNotification posted : notifications().getActiveNotifications()) {
            if (posted.getId() == TRAY_ID && packageName().equals(posted.getPackageName())) {
                return posted.getNotification().actions == null
                        ? 0
                        : posted.getNotification().actions.length;
            }
        }
        return 0;
    }

    /** Cancel everything this app posted, so a later assertion sees only what it caused. */
    static void cancelNotifications() {
        notifications().cancelAll();
    }

    private static NotificationManager notifications() {
        return InstrumentationRegistry.getInstrumentation()
                .getTargetContext()
                .getSystemService(NotificationManager.class);
    }
}
