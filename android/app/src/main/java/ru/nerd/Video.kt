package ru.nerd

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.Surface
import java.util.concurrent.ConcurrentHashMap

// Что показывает кадр видео: область монитора, курсор Windows и возраст картинки
class VideoMeta(
    val x: Float,
    val y: Float,
    val w: Float,
    val h: Float,
    val cursorX: Float,
    val cursorY: Float,
    val monW: Int,
    val monH: Int,
    val lagMs: Long,
)

// Системный декодер H.264: картинка сразу выводится на поверхность экрана
class VideoDecoder(surface: Surface, val width: Int, val height: Int, private val onShown: (VideoMeta) -> Unit) {
    private val codec = MediaCodec.createDecoderByType("video/avc")
    private val metas = ConcurrentHashMap<Long, VideoMeta>()
    @Volatile private var running = true
    private val main = Handler(Looper.getMainLooper())

    init {
        val format = MediaFormat.createVideoFormat("video/avc", width, height)
        format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 2 * 1024 * 1024)
        format.setInteger(MediaFormat.KEY_PRIORITY, 0)
        if (Build.VERSION.SDK_INT >= 30) format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        codec.configure(format, surface, null, 0)
        codec.start()
        Thread { drain() }.apply { isDaemon = true; start() }
    }

    // false - декодер остановлен или сломался; кадры терять нельзя: следующие зависят от предыдущих
    fun feed(data: ByteArray, seq: Long, meta: VideoMeta): Boolean {
        metas[seq] = meta
        while (running) {
            try {
                val i = codec.dequeueInputBuffer(10_000)
                if (i < 0) continue
                val buf = codec.getInputBuffer(i) ?: return false
                buf.clear()
                buf.put(data)
                codec.queueInputBuffer(i, 0, data.size, seq, 0)
                return true
            } catch (e: Exception) {
                return false
            }
        }
        return false
    }

    private fun drain() {
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_DISPLAY)
        val info = MediaCodec.BufferInfo()
        while (running) {
            try {
                val i = codec.dequeueOutputBuffer(info, 10_000)
                if (i < 0) continue
                val meta = metas.remove(info.presentationTimeUs)
                if (meta == null) {
                    codec.releaseOutputBuffer(i, true)
                } else {
                    // кадр и его положение на экране меняются в одном месте главного потока, иначе картинка на миг «прыгает»
                    main.post {
                        if (running) onShown(meta)
                        runCatching { codec.releaseOutputBuffer(i, running) }
                    }
                }
                if (metas.size > 64) metas.keys.sorted().dropLast(32).forEach { metas.remove(it) }
            } catch (e: Exception) {
                if (!running) break
                Thread.sleep(5)
            }
        }
    }

    fun stop() {
        running = false
        runCatching { codec.stop() }
        runCatching { codec.release() }
    }
}
