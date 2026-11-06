package org.sunpetal.app;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The launcher: if the app has no game yet (files/rom.nds or files/sunpetal.data), asks for one with
 * the system's file picker and copies it into the app's own folder (where the native code looks),
 * then starts the game. A data file (made by sunpetal-build) is told from a ROM by its first bytes.
 */
public class PickerActivity extends Activity {
    private static final int PICK = 1;
    private TextView text;
    private Button button;

    private File dir() {
        File dir = getExternalFilesDir(null);
        if (dir == null) dir = getFilesDir();
        return dir;
    }

    private static final byte[] DATA_MAGIC = {'S', 'P', 'D', 'A', 'T', 'A', 0, 1};
    private static final byte[] GAME_CODE = {'Y', 'H', 'N', 'P'};

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // the game asked for a different file: show the picker even though a game file exists
        File pick = new File(dir(), "pick.next");
        boolean force = pick.isFile();
        if (force) pick.delete();
        if (!force && (new File(dir(), "rom.nds").isFile() || new File(dir(), "sunpetal.data").isFile())) {
            start();
            return;
        }
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setGravity(Gravity.CENTER);
        box.setPadding(48, 48, 48, 48);
        text = new TextView(this);
        text.setTextSize(18);
        text.setGravity(Gravity.CENTER);
        text.setText("Sunpetal needs the game: the ROM file (.nds) or a sunpetal.data made on a computer. It is copied into the app and stays on this phone.");
        button = new Button(this);
        button.setText("Choose the game file");
        button.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                pick();
            }
        });
        box.addView(text);
        box.addView(button);
        setContentView(box);
        pick();
    }

    private void pick() {
        Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        i.setType("*/*");
        startActivityForResult(i, PICK);
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        if (code != PICK || result != RESULT_OK || data == null || data.getData() == null) {
            if (text != null) text.setText("No file chosen.");
            return;
        }
        final Uri uri = data.getData();
        button.setEnabled(false);
        text.setText("Copying…");
        new Thread(new Runnable() {
            public void run() {
                final String err = copy(uri);
                runOnUiThread(new Runnable() {
                    public void run() {
                        if (err == null) {
                            start();
                        } else {
                            button.setEnabled(true);
                            Toast.makeText(PickerActivity.this, err, Toast.LENGTH_LONG).show();
                            pick();
                        }
                    }
                });
            }
        }).start();
    }

    /** Copies the chosen file to rom.nds or sunpetal.data (through a temporary name); null on success. */
    private String copy(Uri uri) {
        File folder = dir();
        File tmp = new File(folder, "picked.part");
        folder.mkdirs();
        try {
            InputStream in = getContentResolver().openInputStream(uri);
            if (in == null) return "cannot open it";
            OutputStream os = new FileOutputStream(tmp);
            byte[] buf = new byte[1 << 16];
            byte[] head = new byte[0x10];
            long n = 0;
            int k;
            while ((k = in.read(buf)) > 0) {
                if (n < head.length) System.arraycopy(buf, 0, head, (int) n, (int) Math.min(k, head.length - n));
                os.write(buf, 0, k);
                n += k;
            }
            os.close();
            in.close();
            if (n < 0x200) {
                tmp.delete();
                return "that is not a DS ROM or a data file (too small)";
            }
            boolean data = true;
            for (int i = 0; i < DATA_MAGIC.length; i++) {
                if (head[i] != DATA_MAGIC[i]) {
                    data = false;
                    break;
                }
            }
            if (!data) {
                for (int i = 0; i < GAME_CODE.length; i++) {
                    if (head[0xC + i] != GAME_CODE[i]) {
                        tmp.delete();
                        return "not the supported ROM (the game code must be YHNP)";
                    }
                }
            }
            File out = new File(folder, data ? "sunpetal.data" : "rom.nds");
            if (!tmp.renameTo(out)) return "cannot save it";
            // a data file holding a ROM goes before rom.nds: a ROM picked now must not be shadowed by an old one.
            // A data file picked now keeps rom.nds (a data file without a ROM inside needs it).
            if (!data) new File(folder, "sunpetal.data").delete();
            // keep the player's save: move it to the new game's name if the new one has none
            File oldSave = new File(folder, data ? "rom.sav" : "sunpetal.sav");
            File newSave = new File(folder, data ? "sunpetal.sav" : "rom.sav");
            if (!newSave.isFile() && oldSave.isFile()) oldSave.renameTo(newSave);
            return null;
        } catch (Exception e) {
            tmp.delete();
            return String.valueOf(e.getMessage());
        }
    }

    private void start() {
        startActivity(new Intent(this, android.app.NativeActivity.class));
        finish();
    }
}
