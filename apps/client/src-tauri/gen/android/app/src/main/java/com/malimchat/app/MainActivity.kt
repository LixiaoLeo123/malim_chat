package com.malimchat.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    WindowCompat.setDecorFitsSystemWindows(window, true)
    super.onCreate(savedInstanceState)
  }
}
