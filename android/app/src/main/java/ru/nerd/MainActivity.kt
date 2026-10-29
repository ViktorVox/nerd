package ru.nerd

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.LinkedBlockingQueue
import kotlin.math.cos
import kotlin.math.sin

private const val PORT = 47800
private const val MAGIC: Byte = 0x4E

// Те же действия, что в pc/src/main.rs
private const val HOVER: Byte = 0
private const val DOWN: Byte = 1
private const val DRAG: Byte = 2
private const val UP: Byte = 3
private const val LEAVE: Byte = 4

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
    fun send(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int, tiltY: Int) {
        val b = ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
        b.put(MAGIC).put(action).put(0).put(0)
        b.putFloat(x).putFloat(y).putFloat(pressure)
        b.putShort(tiltX.toShort()).putShort(tiltY.toShort())
        queue.offer(b.array())
    }
}

class PadView(context: Context, private val sender: Sender) : View(context) {
    private fun emit(action: Byte, e: MotionEvent, i: Int, h: Int) {
        val x = if (h < 0) e.getX(i) else e.getHistoricalX(i, h)
        val y = if (h < 0) e.getY(i) else e.getHistoricalY(i, h)
        val p = if (h < 0) e.getPressure(i) else e.getHistoricalPressure(i, h)
        val tilt = e.getAxisValue(MotionEvent.AXIS_TILT, i)
        val ori = e.getAxisValue(MotionEvent.AXIS_ORIENTATION, i)
        val deg = Math.toDegrees(tilt.toDouble())
        sender.send(
            action,
            (x / width).coerceIn(0f, 1f),
            (y / height).coerceIn(0f, 1f),
            p,
            (deg * sin(ori.toDouble())).toInt(),
            (deg * cos(ori.toDouble())).toInt(),
        )
    }

    private fun handle(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> emit(DOWN, e, 0, -1)
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until e.historySize) emit(DRAG, e, 0, h)
                emit(DRAG, e, 0, -1)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> emit(UP, e, 0, -1)
            MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> emit(HOVER, e, 0, -1)
            MotionEvent.ACTION_HOVER_EXIT -> emit(LEAVE, e, 0, -1)
            else -> return false
        }
        return true
    }

    override fun onTouchEvent(e: MotionEvent) = handle(e)
    override fun onGenericMotionEvent(e: MotionEvent) = handle(e)
}

class MainActivity : Activity() {
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
            val pad = PadView(this, Sender(host))
            pad.layoutParams = ViewGroup.LayoutParams(-1, -1)
            setContentView(pad)
            @Suppress("DEPRECATION")
            pad.systemUiVisibility = View.SYSTEM_UI_FLAG_FULLSCREEN or
                View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
        }
    }
}
