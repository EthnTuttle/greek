package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class HomeState(
    val dueCardsCount: Int = 12,
    val streak: Int = 7,
    val hasRecentReading: Boolean = true
)

class HomeViewModel : ViewModel() {
    private val _state = MutableStateFlow(HomeState())
    val state: StateFlow<HomeState> = _state.asStateFlow()

    // Will be replaced with actual Rust integration
    fun refreshStats() {
        // Mock: In real app, this will call Rust library
        // val stats = GreekCore.getHomeStats()
        // _state.value = HomeState(
        //     dueCardsCount = stats.dueCount,
        //     streak = stats.streak,
        //     hasRecentReading = stats.lastReadingTimestamp != null
        // )
    }
}
