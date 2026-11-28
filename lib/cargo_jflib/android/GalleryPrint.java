/* Printing for plat (lib/plat/android_print.rs; docs/ANDROID.md AN4): the PDF plat made of the job
 * goes to the system's PrintManager through a PrintDocumentAdapter that hands it over as it is; the
 * system's dialog (a printer or "Save as PDF", paper, copies, pages, a preview) does the rest.
 * - call CALL_PRINT -> u32 1: there is a print service (PrintManager) and the activity;
 * - request REQUEST_PRINT: the title, u32 paper (plat::PAPER_*), u32 flags (PJF_LANDSCAPE), u32 pages
 *   (0 unknown), then the PDF -> false (refused at once, plat reads ERR_UNSUPPORTED) or true; the
 *   answer comes when the dialog is done (onFinish) through GalleryHost.nativeResult: DONE (the
 *   system took the job: queued, printing, saved), CANCELLED, FAILED.
 * The job's paper and orientation are the dialog's first choice; the PDF waits in the cache
 * (jfgui-print/) until the adapter is finished with it.  PrintManager.print runs on the UI thread. */
package org.jflib.gallery;

import android.app.Activity;
import android.content.Context;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.print.PageRange;
import android.print.PrintAttributes;
import android.print.PrintDocumentAdapter;
import android.print.PrintDocumentInfo;
import android.print.PrintJob;
import android.print.PrintJobInfo;
import android.print.PrintManager;
import android.util.Log;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicInteger;

final class GalleryPrint {
    // android_print.rs, web.rs
    static final int CALL_PRINT = 70, REQUEST_PRINT = 35;
    // plat::PAPER_*, plat::PJF_LANDSCAPE
    static final int PAPER_A4 = 0, PAPER_LETTER = 1, PAPER_A5 = 2, PAPER_LEGAL = 3, PAPER_A3 = 4, PJF_LANDSCAPE = 1;
    private static final String TAG = "jfgui";
    // the jobs' files (print may come from any thread)
    private static final AtomicInteger next = new AtomicInteger();

    /** CALL_PRINT: u32 1 with a print service and an activity, else 0. */
    static byte[] call(Activity a) {
        boolean ok = a != null && a.getSystemService(Context.PRINT_SERVICE) != null;
        return new byte[] {(byte)(ok ? 1 : 0), 0, 0, 0};
    }

    private static PrintAttributes.MediaSize media(int paper) {
        switch (paper) {
            case PAPER_LETTER: return PrintAttributes.MediaSize.NA_LETTER;
            case PAPER_A5: return PrintAttributes.MediaSize.ISO_A5;
            case PAPER_LEGAL: return PrintAttributes.MediaSize.NA_LEGAL;
            case PAPER_A3: return PrintAttributes.MediaSize.ISO_A3;
            default: return PrintAttributes.MediaSize.ISO_A4;
        }
    }

    /** REQUEST_PRINT (see the file header). */
    static boolean request(Activity a, int id, byte[] payload) {
        if (a == null || payload == null || payload.length < 16) return false;
        PrintManager pm = (PrintManager)a.getSystemService(Context.PRINT_SERVICE);
        if (pm == null) return false;
        ByteBuffer b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN);
        int n = b.getInt();
        if (n < 0 || n > b.remaining() - 12) return false;
        byte[] t = new byte[n];
        b.get(t);
        String title = new String(t, StandardCharsets.UTF_8);
        int paper = b.getInt(), flags = b.getInt(), pages = b.getInt();
        File dir = new File(a.getCacheDir(), "jfgui-print");
        if (!dir.isDirectory() && !dir.mkdirs()) return false;
        File pdf = new File(dir, "job-" + (next.getAndIncrement() & 0xFFFF) + ".pdf");
        try (FileOutputStream out = new FileOutputStream(pdf)) {
            out.write(payload, b.position(), b.remaining());
        } catch (Exception e) {
            Log.w(TAG, "print: " + e);
            pdf.delete();
            return false;
        }
        PrintAttributes.MediaSize size = media(paper);
        if ((flags & PJF_LANDSCAPE) != 0) size = size.asLandscape();
        PrintAttributes attrs = new PrintAttributes.Builder().setMediaSize(size)
            .setColorMode(PrintAttributes.COLOR_MODE_COLOR).setMinMargins(PrintAttributes.Margins.NO_MARGINS).build();
        String name = title.isEmpty() ? "Dokument" : title;
        Adapter adapter = new Adapter(id, pdf, name, pages);
        Runnable start = () -> {
            try {
                adapter.job = pm.print(name, adapter, attrs);
            } catch (Exception e) {
                Log.w(TAG, "print: " + e);
                adapter.answer(GalleryHost.FAILED);
                pdf.delete();
            }
        };
        if (Looper.myLooper() == Looper.getMainLooper()) start.run();
        else a.runOnUiThread(start);
        return true;
    }

    /** Hands plat's PDF over as it is (its pages, whatever the dialog's paper: the spooler fits them). */
    private static final class Adapter extends PrintDocumentAdapter {
        final int id, pages;
        final File pdf;
        final String name;
        PrintJob job;
        private boolean answered;

        Adapter(int id, File pdf, String name, int pages) { this.id = id; this.pdf = pdf; this.name = name; this.pages = pages; }

        void answer(int status) {
            if (answered) return;
            answered = true;
            GalleryHost.nativeResult(id, status, null);
        }

        @Override public void onLayout(PrintAttributes before, PrintAttributes now, CancellationSignal cancel,
                LayoutResultCallback callback, Bundle extras) {
            if (cancel.isCanceled()) {
                callback.onLayoutCancelled();
                return;
            }
            PrintDocumentInfo info = new PrintDocumentInfo.Builder(name.endsWith(".pdf") ? name : name + ".pdf")
                .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
                .setPageCount(pages > 0 ? pages : PrintDocumentInfo.PAGE_COUNT_UNKNOWN).build();
            // the document stays the same; only a first layout or new attributes count as a change
            callback.onLayoutFinished(info, before == null || !before.equals(now));
        }

        @Override public void onWrite(PageRange[] ranges, ParcelFileDescriptor destination, CancellationSignal cancel,
                WriteResultCallback callback) {
            // (the descriptor is the spooler's: written, not closed here)
            FileOutputStream out = new FileOutputStream(destination.getFileDescriptor());
            try (InputStream in = new FileInputStream(pdf)) {
                byte[] buf = new byte[1 << 16];
                for (int n; (n = in.read(buf)) > 0; ) {
                    if (cancel.isCanceled()) {
                        callback.onWriteCancelled();
                        return;
                    }
                    out.write(buf, 0, n);
                }
                out.flush();
                callback.onWriteFinished(new PageRange[] {PageRange.ALL_PAGES});
            } catch (Exception e) {
                Log.w(TAG, "print: " + e);
                callback.onWriteFailed(e.toString());
            }
        }

        // The dialog is done: a job still CREATED never went anywhere (closed with Back: the
        // spooler forgets it, so its state stays the one cached at print()), CANCELED was
        // cancelled, FAILED failed; QUEUED, STARTED, BLOCKED and COMPLETED (saved as a PDF) are the
        // system's now
        @Override public void onFinish() {
            pdf.delete();
            int state = job == null ? PrintJobInfo.STATE_CREATED : job.getInfo().getState();
            int status;
            switch (state) {
                case PrintJobInfo.STATE_CREATED: case PrintJobInfo.STATE_CANCELED: status = GalleryHost.CANCELLED; break;
                case PrintJobInfo.STATE_FAILED: status = GalleryHost.FAILED; break;
                default: status = GalleryHost.DONE;
            }
            Log.i(TAG, "print: " + name + " done, job " + state + " -> " + status);
            answer(status);
        }
    }
}
