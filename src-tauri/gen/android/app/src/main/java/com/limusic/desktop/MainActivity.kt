package com.limusic.desktop

import android.os.Bundle

class MainActivity : WryActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AudioBridge.attachContext(this)
    }
}
