package ru.nerd

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.Bundle
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.io.DataInputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.LinkedBlockingQueue
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.sin

private const val PORT = 47800
private const val SCREEN_PORT = 47801
private const val JPEG_QUALITY = 60
private const val MAGIC: Byte = 0x4E
private const val CMD_MAGIC: Byte = 0x4F

// Те же действия, что в pc/src/main.rs
private const val HOVER: Byte = 0
private const val DOWN: Byte = 1
private const val DRAG: Byte = 2
private const val UP: Byte = 3
private const val LEAVE: Byte = 4

private const val FINGER_PRESSURE = 0.6f
private const val TAP_SLOP = 24f
private const val LONG_PRESS_MS = 500L

class Sender(host: String) {
    private val queue = LinkedBlockingQueue<ByteArray>()
    private val socket = DatagramSocket()
    private val address = InetAddress.getByName(host)

    init {
        Thread {
            while (true) {
                val b = queue.take()
                socket.send(DatagramPacket(b, b.size, address, PORT))
            }
        }.apply { isDaemon = true }.start()
    }

    // magic u8 | action u8 | buttons u8 | pad u8 | x f32 | y f32 | pressure f32 | tilt_x i16 | tilt_y i16
    fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int, tiltY: Int) {
        val b = ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
        b.put(MAGIC).put(action).put(0).put(0)
        b.putFloat(x).putFloat(y).putFloat(pressure)
        b.putShort(tiltX.toShort()).putShort(tiltY.toShort())
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 | pad | pad | x f32 | y f32 (kind 0: курсор)
    fun mouseMove(x: Float, y: Float) {
        val b = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        b.put(CMD_MAGIC).put(0).put(0).put(0).putFloat(x).putFloat(y)
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 (1: кнопка) | кнопка u8 (0 левая, 1 правая) | down u8
    fun mouseButton(button: Int, down: Boolean) {
        queue.offer(byteArrayOf(CMD_MAGIC, 1, button.toByte(), if (down) 1 else 0))
    }
}

// Кадр экрана ПК и область монитора (доли 0..1), которую он показывает
class Frame(val bmp: Bitmap, val x: Float, val y: Float, val w: Float, val h: Float)

class ScreenClient(private val host: String, private val pad: PadView) {
    @Volatile private var running = true
    @Volatile private var socket: Socket? = null

    init {
        Thread { loop() }.apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        socket?.close()
    }

    private fun loop() {
        while (running) {
            val s = Socket()
            try {
                socket = s
                s.tcpNoDelay = true
                s.connect(InetSocketAddress(host, SCREEN_PORT), 3000)
                val input = DataInputStream(s.getInputStream().buffered(1 shl 16))
                val out = s.getOutputStream()
                val head = ByteArray(24)
                while (running) {
                    out.write(pad.screenRequest())
                    out.flush()
                    input.readFully(head)
                    val b = ByteBuffer.wrap(head).order(ByteOrder.LITTLE_ENDIAN)
                    val x = b.float; val y = b.float; val w = b.float; val h = b.float
                    val monW = b.short.toInt() and 0xFFFF
                    val monH = b.short.toInt() and 0xFFFF
                    val len = b.int
                    if (len <= 0 || len > 20_000_000) break
                    val data = ByteArray(len)
                    input.readFully(data)
                    val bmp = BitmapFactory.decodeByteArray(data, 0, len)
                    if (bmp != null) pad.onFrame(Frame(bmp, x, y, w, h), monW.toFloat(), monH.toFloat())
                }
            } catch (e: Exception) {
                // нет связи: пробуем снова
            } finally {
                runCatching { s.close() }
            }
            if (running) Thread.sleep(1000)
        }
    }
}

enum class Mode(val title: String) { DRAW("Рисовать"), MOUSE("Мышь") }

class PadView(context: Context, private val sender: Sender) : View(context) {
    var mode = Mode.DRAW

    // Размер монитора ПК; позже придёт от ПК вместе с картинкой
    private var monW = 1920f
    private var monH = 1080f

    @Volatile private var frame: Frame? = null

    // Вид: scale — пикселей экрана на пиксель монитора; (ox, oy) — точка монитора в левом верхнем углу экрана
    private var scale = 1f
    private var ox = 0f
    private var oy = 0f
    private var fitScale = 1f

    private var gesturing = false
    private var gDist = 1f
    private var gScale = 1f
    private var gAnchorX = 0f
    private var gAnchorY = 0f

    private var penDown = false
    private var lastX = 0f
    private var lastY = 0f
    private var downTime = 0L
    private var downX = 0f
    private var downY = 0f
    private var moved = false

    private val grid = Paint().apply { color = Color.rgb(60, 60, 70); strokeWidth = 1f }
    private val border = Paint().apply {
        color = Color.rgb(120, 120, 140); strokeWidth = 3f; style = Paint.Style.STROKE
    }
    private val bitmapPaint = Paint().apply { isFilterBitmap = true }
    private val text = Paint().apply { color = Color.WHITE; textSize = 32f; isAntiAlias = true }

    fun fit() {
        fitScale = min(width / monW, height / monH)
        scale = fitScale
        ox = -(width / scale - monW) / 2
        oy = -(height / scale - monH) / 2
        invalidate()
    }

    // Запрос к ПК (24 байта): видимая область монитора и размер картинки в пикселях экрана
    fun screenRequest(): ByteArray {
        val x0 = ox.coerceAtLeast(0f)
        val y0 = oy.coerceAtLeast(0f)
        val x1 = (ox + width / scale).coerceAtMost(monW)
        val y1 = (oy + height / scale).coerceAtMost(monH)
        val b = ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
        b.putFloat(x0 / monW).putFloat(y0 / monH).putFloat((x1 - x0) / monW).putFloat((y1 - y0) / monH)
        b.putShort(((x1 - x0) * scale).toInt().coerceIn(16, 2560).toShort())
        b.putShort(((y1 - y0) * scale).toInt().coerceIn(16, 1440).toShort())
        b.put(JPEG_QUALITY.toByte())
        return b.array()
    }

    fun onFrame(f: Frame, w: Float, h: Float) {
        frame = f
        if (w != monW || h != monH) {
            monW = w; monH = h
            post { fit() }
        }
        postInvalidate()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = fit()

    private fun nx(x: Float) = ((ox + x / scale) / monW).coerceIn(0f, 1f)
    private fun ny(y: Float) = ((oy + y / scale) / monH).coerceIn(0f, 1f)

    override fun onDraw(c: Canvas) {
        c.drawColor(Color.rgb(30, 30, 36))
        for (i in 0..16) {
            val x = (i * monW / 16 - ox) * scale
            c.drawLine(x, 0f, x, height.toFloat(), grid)
        }
        for (i in 0..9) {
            val y = (i * monH / 9 - oy) * scale
            c.drawLine(0f, y, width.toFloat(), y, grid)
        }
        frame?.let {
            val dst = RectF(
                (it.x * monW - ox) * scale, (it.y * monH - oy) * scale,
                ((it.x + it.w) * monW - ox) * scale, ((it.y + it.h) * monH - oy) * scale,
            )
            c.drawBitmap(it.bmp, null, dst, bitmapPaint)
        }
        c.drawRect(-ox * scale, -oy * scale, (monW - ox) * scale, (monH - oy) * scale, border)
        c.drawText("Увеличение ×%.1f".format(scale / fitScale), 24f, height - 24f, text)
    }

    private fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int = 0, tiltY: Int = 0) =
        sender.pen(action, nx(x), ny(y), pressure, tiltX, tiltY)

    // Стилус всегда рисует; тип «палец» зависит от режима
    private fun stylus(e: MotionEvent): Boolean {
        val tilt = Math.toDegrees(e.getAxisValue(MotionEvent.AXIS_TILT).toDouble())
        val ori = e.getAxisValue(MotionEvent.AXIS_ORIENTATION).toDouble()
        val tx = (tilt * sin(ori)).toInt()
        val ty = (tilt * cos(ori)).toInt()
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> pen(DOWN, e.x, e.y, e.pressure, tx, ty)
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until e.historySize) {
                    pen(DRAG, e.getHistoricalX(h), e.getHistoricalY(h), e.getHistoricalPressure(h), tx, ty)
                }
                pen(DRAG, e.x, e.y, e.pressure, tx, ty)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> pen(UP, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> pen(HOVER, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_EXIT -> pen(LEAVE, e.x, e.y, 0f, tx, ty)
            else -> return false
        }
        return true
    }

    private fun fingerDown(e: MotionEvent) {
        lastX = e.x; lastY = e.y; downX = e.x; downY = e.y
        downTime = e.eventTime; moved = false
        when (mode) {
            Mode.DRAW -> { pen(DOWN, e.x, e.y, FINGER_PRESSURE); penDown = true }
            Mode.MOUSE -> sender.mouseMove(nx(e.x), ny(e.y))
        }
    }

    private fun fingerMove(e: MotionEvent) {
        if (hypot(e.x - downX, e.y - downY) > TAP_SLOP) moved = true
        lastX = e.x; lastY = e.y
        when (mode) {
            Mode.DRAW -> {
                for (h in 0 until e.historySize) {
                    pen(DRAG, e.getHistoricalX(h), e.getHistoricalY(h), FINGER_PRESSURE)
                }
                pen(DRAG, e.x, e.y, FINGER_PRESSURE)
            }
            Mode.MOUSE -> sender.mouseMove(nx(e.x), ny(e.y))
        }
    }

    // up = true: палец отпущен; false: отменён вторым пальцем (клик не нужен)
    private fun fingerEnd(e: MotionEvent?, up: Boolean) {
        when (mode) {
            Mode.DRAW -> if (penDown) {
                pen(UP, lastX, lastY, 0f)
                pen(LEAVE, lastX, lastY, 0f)
                penDown = false
            }
            Mode.MOUSE -> if (up && e != null && !moved) {
                val button = if (e.eventTime - downTime >= LONG_PRESS_MS) 1 else 0
                sender.mouseButton(button, true)
                sender.mouseButton(button, false)
            }
        }
    }

    private fun gestureStart(e: MotionEvent) {
        gDist = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1)).coerceAtLeast(1f)
        gScale = scale
        val mx = (e.getX(0) + e.getX(1)) / 2
        val my = (e.getY(0) + e.getY(1)) / 2
        gAnchorX = ox + mx / scale
        gAnchorY = oy + my / scale
    }

    private fun gestureMove(e: MotionEvent) {
        val d = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1))
        val mx = (e.getX(0) + e.getX(1)) / 2
        val my = (e.getY(0) + e.getY(1)) / 2
        scale = (gScale * d / gDist).coerceIn(fitScale * 0.9f, fitScale * 12f)
        ox = gAnchorX - mx / scale
        oy = gAnchorY - my / scale
        invalidate()
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (e.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS) return stylus(e)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> { gesturing = false; fingerDown(e) }
            MotionEvent.ACTION_POINTER_DOWN -> {
                if (!gesturing) {
                    fingerEnd(null, false)
                    gesturing = true
                }
                if (e.pointerCount == 2) gestureStart(e)
            }
            MotionEvent.ACTION_MOVE -> when {
                gesturing -> if (e.pointerCount >= 2) gestureMove(e)
                else -> fingerMove(e)
            }
            MotionEvent.ACTION_POINTER_UP -> if (gesturing && e.pointerCount == 3) {
                // Остались два пальца: пересчитать опору жеста
                gestureStart(e)
            }
            MotionEvent.ACTION_UP -> { if (!gesturing) fingerEnd(e, true); gesturing = false }
            MotionEvent.ACTION_CANCEL -> { if (!gesturing) fingerEnd(e, false); gesturing = false }
            else -> return false
        }
        return true
    }

    override fun onGenericMotionEvent(e: MotionEvent): Boolean =
        if (e.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS) stylus(e) else false
}

class MainActivity : Activity() {
    private var screen: ScreenClient? = null

    override fun onDestroy() {
        screen?.stop()
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = getSharedPreferences("nerd", MODE_PRIVATE)

        val ip = EditText(this).apply {
            hint = "IP компьютера, например 192.168.1.10"
            setText(prefs.getString("ip", ""))
        }
        val start = Button(this).apply { text = "Подключиться" }
        val panel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
            addView(ip)
            addView(start)
        }
        setContentView(panel)

        start.setOnClickListener {
            val host = ip.text.toString().trim()
            if (host.isEmpty()) return@setOnClickListener
            prefs.edit().putString("ip", host).apply()
            showPad(host)
        }
    }

    private fun showPad(host: String) {
        val pad = PadView(this, Sender(host))
        screen = ScreenClient(host, pad)
        val modeButton = Button(this).apply { text = pad.mode.title }
        modeButton.setOnClickListener {
            pad.mode = if (pad.mode == Mode.DRAW) Mode.MOUSE else Mode.DRAW
            modeButton.text = pad.mode.title
        }
        val resetButton = Button(this).apply { text = "Вид" }
        resetButton.setOnClickListener { pad.fit() }
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(modeButton)
            addView(resetButton)
        }
        val root = FrameLayout(this).apply {
            addView(pad, FrameLayout.LayoutParams(-1, -1))
            addView(bar, FrameLayout.LayoutParams(-2, -2, Gravity.TOP or Gravity.START))
        }
        setContentView(root)
        @Suppress("DEPRECATION")
        root.systemUiVisibility = View.SYSTEM_UI_FLAG_FULLSCREEN or
            View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
    }
}
