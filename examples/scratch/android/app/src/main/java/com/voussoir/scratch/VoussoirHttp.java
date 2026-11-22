package com.voussoir.scratch;

import android.util.Log;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;

/**
 * HTTP for the updater, standing in for the desktop TLS stack.
 *
 * <p>{@code HttpURLConnection} is what makes this worth its own class: a bundled
 * stack would have to be taught the system certificate store separately, and
 * Android's store is updated outside the app.
 *
 * <p>{@link #get(String)} blocks, so it must stay off the UI thread. Rust calls it
 * from the updater's worker thread, which is what makes the blocking call safe.
 */
final class VoussoirHttp {
    private static final String TAG = MainActivity.TAG;

    private VoussoirHttp() {}

    /**
     * The largest body an update check will read, matching the Rust side's cap.
     *
     * <p>The endpoint answers with a version and a note; reading an unbounded
     * body lets one bad endpoint cost an unbounded allocation on the UI-thread
     * hop's worker.
     */
    private static final int MAX_BODY = 64 * 1024;

    /** The body of {@code url}, or null if it could not be fetched. */
    static String get(String url) {
        HttpURLConnection connection = null;
        try {
            connection = (HttpURLConnection) new URL(url).openConnection();
            connection.setConnectTimeout(15000);
            connection.setReadTimeout(30000);
            connection.setRequestMethod("GET");
            int status = connection.getResponseCode();
            if (status < 200 || status >= 300) {
                Log.w(TAG, "update check: " + url + " answered " + status);
                return null;
            }
            try (InputStream body = connection.getInputStream()) {
                return new String(readAll(body), StandardCharsets.UTF_8);
            }
        } catch (IOException e) {
            Log.w(TAG, "update check: " + url + " failed: " + e.getMessage());
            return null;
        } finally {
            if (connection != null) {
                connection.disconnect();
            }
        }
    }

    static byte[] readAll(InputStream input) throws IOException {
        ByteArrayOutputStream buffer = new ByteArrayOutputStream();
        byte[] chunk = new byte[8192];
        int read;
        while ((read = input.read(chunk)) > 0) {
            if (buffer.size() + read > MAX_BODY) {
                throw new IOException("update check body exceeds " + MAX_BODY + " bytes");
            }
            buffer.write(chunk, 0, read);
        }
        return buffer.toByteArray();
    }
}
