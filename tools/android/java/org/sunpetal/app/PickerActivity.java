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

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (new File(dir(), "rom.nds").isFile() || new File(dir(), "sunpetal.data").isFile()) {
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
                            text.setText("Could not copy the file: " + err);
                            button.setEnabled(true);
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
            byte[] head = new byte[8];
            long n = 0;
            int k;
            while ((k = in.read(buf)) > 0) {
                if (n < 8) System.arraycopy(buf, 0, head, (int) n, (int) Math.min(k, 8 - n));
                os.write(buf, 0, k);
                n += k;
            }
            os.close();
            in.close();
            if (n < 0x200) {
                tmp.delete();
                return "that is not a DS ROM or a data file (too small)";
            }
            File out = new File(folder, java.util.Arrays.equals(head, DATA_MAGIC) ? "sunpetal.data" : "rom.nds");
            if (!tmp.renameTo(out)) return "cannot save it";
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
