package com.voussoir.scratch;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.IOException;

/**
 * The update body cap, which is the mobile half of the bound the Rust updater
 * sets. The endpoint answers with a version and a note, so a body past the cap
 * is not that document and must not be buffered.
 */
public class VoussoirHttpTest {
    @Test
    public void aBodyUnderTheCapIsRead() throws Exception {
        byte[] body = new byte[4096];
        for (int i = 0; i < body.length; i++) {
            body[i] = (byte) 'x';
        }
        assertArrayEquals(body, VoussoirHttp.readAll(new ByteArrayInputStream(body)));
    }

    @Test
    public void aBodyOverTheCapIsRefused() {
        byte[] body = new byte[64 * 1024 + 1];
        try {
            VoussoirHttp.readAll(new ByteArrayInputStream(body));
            fail("an unbounded body must be refused before it is buffered");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("exceeds"));
        }
    }
}
