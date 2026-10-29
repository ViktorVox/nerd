package ru.nerd

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.sin

private const val FINGER_PRESSURE = 0.6f
private const val TAP_SLOP = 24f
private const val LONG_PRESS_MS = 500L

// Локальная линия живёт чуть дольше, чем нужно кадру ПК, чтобы Krita успела её дорисовать
private const val INK_MARGIN_MS = 80L
private const val INK_MAX_AGE_MS = 3000L

enum class Mode(val title: String) { DRAW("Рисовать"), MOUSE("Мышь") }

// Точка локальной линии в координатах монитора; start = начало нового штриха
private class InkPoint(val x: Float, val y: Float, val t: Long, var start: Boolean)

class PadView(context: Context, private val sender: Sender) : View(context) {
    var mode = Mode.DRAW

    // Размер монитора ПК; уточняется по первому кадру
    private var monW = 1920f
    private var monH = 1080f

    @Volatile private var frame: Frame? = null

    // Вид: scale — пикселей экрана на пиксель монитора; (ox, oy) — точка монитора в левом верхнем углу экрана
    private var scale = 1f
    private var ox = 0f
    private var oy = 0f
    private var fitScale = 1f

    val zoom: Float get() = if (fitScale > 0f) scale / fitScale else 1f

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

    private val ink = ArrayList<InkPoint>()
    private val inkPaint = Paint().apply {
        color = Color.argb(230, 24, 24, 28)
        strokeWidth = 4f
        strokeCap = Paint.Cap.ROUND
        isAntiAlias = true
    }
    private val bitmapPaint = Paint().apply { isFilterBitmap = true }

    init {
        setBackgroundColor(Color.rgb(14, 15, 19))
    }

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

    // sentAt — момент, когда был отправлен запрос этого кадра
    fun onFrame(f: Frame, sentAt: Long, w: Float, h: Float) {
        frame = f
        pruneInk(sentAt - INK_MARGIN_MS)
        if (w != monW || h != monH) {
            monW = w; monH = h
            post { fit() }
        }
        postInvalidate()
    }

    private fun pruneInk(before: Long) {
        synchronized(ink) {
            val old = SystemClock.uptimeMillis() - INK_MAX_AGE_MS
            ink.removeAll { it.t < before || it.t < old }
            ink.firstOrNull()?.start = true
        }
    }

    private fun addInk(x: Float, y: Float, start: Boolean) {
        synchronized(ink) { ink.add(InkPoint(ox + x / scale, oy + y / scale, SystemClock.uptimeMillis(), start)) }
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = fit()

    private fun nx(x: Float) = ((ox + x / scale) / monW).coerceIn(0f, 1f)
    private fun ny(y: Float) = ((oy + y / scale) / monH).coerceIn(0f, 1f)

    override fun onDraw(c: Canvas) {
        frame?.let {
            val dst = RectF(
                (it.x * monW - ox) * scale, (it.y * monH - oy) * scale,
                ((it.x + it.w) * monW - ox) * scale, ((it.y + it.h) * monH - oy) * scale,
            )
            c.drawBitmap(it.bmp, null, dst, bitmapPaint)
        }
        synchronized(ink) {
            var prev: InkPoint? = null
            for (p in ink) {
                if (prev != null && !p.start) {
                    c.drawLine((prev.x - ox) * scale, (prev.y - oy) * scale, (p.x - ox) * scale, (p.y - oy) * scale, inkPaint)
                }
                prev = p
            }
        }
    }

    private fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int = 0, tiltY: Int = 0) =
        sender.pen(action, nx(x), ny(y), pressure, tiltX, tiltY)

    // Стилус всегда рисует; палец зависит от режима
    private fun stylus(e: MotionEvent): Boolean {
        val tilt = Math.toDegrees(e.getAxisValue(MotionEvent.AXIS_TILT).toDouble())
        val ori = e.getAxisValue(MotionEvent.AXIS_ORIENTATION).toDouble()
        val tx = (tilt * sin(ori)).toInt()
        val ty = (tilt * cos(ori)).toInt()
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pen(DOWN, e.x, e.y, e.pressure, tx, ty)
                addInk(e.x, e.y, true)
            }
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until e.historySize) {
                    pen(DRAG, e.getHistoricalX(h), e.getHistoricalY(h), e.getHistoricalPressure(h), tx, ty)
                    addInk(e.getHistoricalX(h), e.getHistoricalY(h), false)
                }
                pen(DRAG, e.x, e.y, e.pressure, tx, ty)
                addInk(e.x, e.y, false)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> pen(UP, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> pen(HOVER, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_EXIT -> pen(LEAVE, e.x, e.y, 0f, tx, ty)
            else -> return false
        }
        invalidate()
        return true
    }

    private fun fingerDown(e: MotionEvent) {
        lastX = e.x; lastY = e.y; downX = e.x; downY = e.y
        downTime = e.eventTime; moved = false
        when (mode) {
            Mode.DRAW -> {
                pen(DOWN, e.x, e.y, FINGER_PRESSURE)
                addInk(e.x, e.y, true)
                penDown = true
            }
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
                    addInk(e.getHistoricalX(h), e.getHistoricalY(h), false)
                }
                pen(DRAG, e.x, e.y, FINGER_PRESSURE)
                addInk(e.x, e.y, false)
                invalidate()
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
