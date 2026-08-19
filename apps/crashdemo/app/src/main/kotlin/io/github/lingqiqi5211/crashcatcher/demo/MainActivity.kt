package io.github.lingqiqi5211.crashcatcher.demo

import android.os.Bundle
import android.os.Process
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                CrashDemoScreen(
                    javaCrash = { error("CrashDemo Java exception") },
                    swallowedCrash = ::triggerSwallowedCrash,
                    anr = { Thread.sleep(30_000) },
                    nativeCrash = { Process.sendSignal(Process.myPid(), 11) },
                    wtf = { Log.wtf("CrashCatcherDemo", "CrashDemo WTF report") },
                )
            }
        }
    }

    private fun triggerSwallowedCrash() {
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { _, throwable ->
            Log.i("CrashCatcherDemo", "Custom handler swallowed ${throwable.javaClass.name}")
            Thread.setDefaultUncaughtExceptionHandler(previous)
        }
        Thread({ error("CrashDemo self-handled exception") }, "self-handled-demo").start()
    }
}

@Composable
private fun CrashDemoScreen(
    javaCrash: () -> Unit,
    swallowedCrash: () -> Unit,
    anr: () -> Unit,
    nativeCrash: () -> Unit,
    wtf: () -> Unit,
) {
    Scaffold(
        topBar = { TopAppBar(title = { Text("CrashCatcher Demo") }) },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentPadding = PaddingValues(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Text(
                    text = "Each action deliberately exercises one collector path. " +
                        "Run this only on a test device.",
                    style = MaterialTheme.typography.bodyLarge,
                )
            }
            item { DemoButton("Java exception", javaCrash) }
            item { DemoButton("Self-handled exception", swallowedCrash) }
            item { DemoButton("ANR (blocks for 30 seconds)", anr) }
            item { DemoButton("Native SIGSEGV", nativeCrash) }
            item { DemoButton("WTF report", wtf) }
        }
    }
}

@Composable
private fun DemoButton(label: String, action: () -> Unit) {
    Button(
        onClick = action,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(label)
    }
}
