package ru.nerd

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlinx.coroutines.delay

private val NerdColors = darkColorScheme(
    primary = Color(0xFF8FA8FF),
    onPrimary = Color(0xFF0B1330),
    background = Color(0xFF0E0F13),
    onBackground = Color(0xFFE6E8EE),
    surface = Color(0xFF181A21),
    onSurface = Color(0xFFE6E8EE),
    onSurfaceVariant = Color(0xFF9AA0B0),
    secondaryContainer = Color(0xFF2B3252),
    onSecondaryContainer = Color(0xFFDDE3FF),
    outline = Color(0xFF3A3E4D),
)

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        val prefs = getSharedPreferences("nerd", MODE_PRIVATE)

        setContent {
            MaterialTheme(colorScheme = NerdColors) {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    var host by remember { mutableStateOf<String?>(null) }
                    // Сразу подключаемся к последнему компьютеру, но не после того, как пользователь сам отключился
                    var auto by remember { mutableStateOf(true) }
                    val current = host
                    if (current == null) {
                        ConnectScreen(last = prefs.getString("ip", "") ?: "", auto = auto) {
                            prefs.edit().putString("ip", it).apply()
                            host = it
                        }
                    } else {
                        PadScreen(current) { auto = false; host = null }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConnectScreen(last: String, auto: Boolean, onConnect: (String) -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val discovery = remember { Discovery(context) }
    DisposableEffect(Unit) { onDispose { discovery.stop() } }

    var found by remember { mutableStateOf(emptyList<FoundPc>()) }
    var manual by remember { mutableStateOf(false) }
    var ip by remember { mutableStateOf(last) }
    LaunchedEffect(Unit) {
        while (true) {
            found = discovery.found
            delay(500)
        }
    }
    LaunchedEffect(found) {
        if (auto && last.isNotEmpty() && found.any { it.ip == last }) onConnect(last)
    }

    Box(modifier = Modifier.fillMaxSize().statusBarsPadding(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.widthIn(max = 460.dp).fillMaxWidth().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Nerd", style = MaterialTheme.typography.displayMedium, color = MaterialTheme.colorScheme.onBackground)
            Text(
                "Телефон как графический планшет для компьютера",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            if (found.isEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    Text(
                        "Ищу компьютеры в сети…",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    "Запусти программу Nerd на компьютере. Телефон и компьютер должны быть в одной Wi-Fi сети.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            found.forEach { pc ->
                Card(
                    onClick = { onConnect(pc.ip) },
                    shape = RoundedCornerShape(20.dp),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column(modifier = Modifier.padding(horizontal = 20.dp, vertical = 16.dp)) {
                        Text(pc.name, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
                        Text(pc.ip, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }

            TextButton(onClick = { manual = !manual }) {
                Text(if (manual) "Скрыть ввод адреса" else "Ввести адрес вручную")
            }
            if (manual) {
                OutlinedTextField(
                    value = ip,
                    onValueChange = { ip = it },
                    label = { Text("IP компьютера") },
                    placeholder = { Text("192.168.1.10") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(
                    onClick = { if (ip.isNotBlank()) onConnect(ip.trim()) },
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.fillMaxWidth().height(52.dp),
                ) { Text("Подключиться") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PadScreen(host: String, onExit: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val pad = remember { PadView(context, Sender(host)) }
    val client = remember { ScreenClient(host, pad) }
    DisposableEffect(Unit) { onDispose { client.stop() } }
    BackHandler(onBack = onExit)

    var mode by remember { mutableStateOf(pad.mode) }
    var stats by remember { mutableStateOf("") }
    var connected by remember { mutableStateOf(false) }
    var zoom by remember { mutableStateOf(1f) }
    LaunchedEffect(Unit) {
        while (true) {
            stats = client.statsText
            connected = client.connected
            zoom = pad.zoom
            delay(400)
        }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        AndroidView(factory = { pad }, modifier = Modifier.fillMaxSize())

        if (!connected) {
            Column(
                modifier = Modifier.align(Alignment.Center),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                CircularProgressIndicator(color = MaterialTheme.colorScheme.primary)
                Text("Подключение к $host…", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }

        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.88f),
            tonalElevation = 6.dp,
            modifier = Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 8.dp),
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                SingleChoiceSegmentedButtonRow {
                    Mode.entries.forEachIndexed { index, m ->
                        SegmentedButton(
                            selected = mode == m,
                            onClick = { mode = m; pad.mode = m },
                            shape = SegmentedButtonDefaults.itemShape(index, Mode.entries.size),
                        ) { Text(m.title) }
                    }
                }
                IconButton(onClick = { pad.fit() }) {
                    Icon(Icons.Rounded.Refresh, contentDescription = "Сбросить вид")
                }
                if (connected) {
                    Text(
                        "×%.1f".format(zoom) + if (stats.isNotEmpty()) " · $stats" else "",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 4.dp),
                    )
                }
                IconButton(onClick = onExit) {
                    Icon(Icons.Rounded.Close, contentDescription = "Отключиться")
                }
            }
        }
    }
}
