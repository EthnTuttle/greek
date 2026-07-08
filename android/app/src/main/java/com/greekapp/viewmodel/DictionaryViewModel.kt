package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class DictionaryEntry(
    val headword: String,
    val pos: String,
    val definition: String,
    val lemma: String = headword
)

data class DictionaryState(
    val query: String = "",
    val results: List<DictionaryEntry> = emptyList(),
    val isSearching: Boolean = false
)

class DictionaryViewModel : ViewModel() {
    private val _state = MutableStateFlow(DictionaryState())
    val state: StateFlow<DictionaryState> = _state.asStateFlow()

    // Mock dictionary data
    private val mockEntries = listOf(
        DictionaryEntry("ἄγω", "verb", "I lead, bring, carry; I go"),
        DictionaryEntry("ἀγρός", "noun m", "field, land, country, farm"),
        DictionaryEntry("ἀδελφός", "noun m", "brother"),
        DictionaryEntry("αἱρέω", "verb", "I take, seize, capture"),
        DictionaryEntry("ἀκούω", "verb", "I hear, listen to (+ gen)"),
        DictionaryEntry("ἀληθής", "adj", "true, real, genuine"),
        DictionaryEntry("ἄλλος", "adj", "other, another, different"),
        DictionaryEntry("ἀνήρ", "noun m", "man, husband, hero (gen: ἀνδρός)"),
        DictionaryEntry("ἄνθρωπος", "noun m", "human being, person, man"),
        DictionaryEntry("ἀπό", "prep", "from, away from (+ gen)"),
        DictionaryEntry("ἀρχή", "noun f", "beginning, origin; rule, power"),
        DictionaryEntry("βάλλω", "verb", "I throw, cast, hurl"),
        DictionaryEntry("βασιλεύς", "noun m", "king, chief (gen: βασιλέως)"),
        DictionaryEntry("γάρ", "particle", "for, because (postpositive)"),
        DictionaryEntry("γε", "particle", "at least, indeed (emphatic)"),
        DictionaryEntry("γῆ", "noun f", "earth, land, ground, country"),
        DictionaryEntry("γίγνομαι", "verb", "I become, happen, am born"),
        DictionaryEntry("γιγνώσκω", "verb", "I know, perceive, understand"),
        DictionaryEntry("γράφω", "verb", "I write, draw, inscribe"),
        DictionaryEntry("δέ", "particle", "but, and, now (postpositive)"),
        DictionaryEntry("δεῖ", "verb", "it is necessary, one must (+ acc & inf)"),
        DictionaryEntry("δείκνυμι", "verb", "I show, point out, prove"),
        DictionaryEntry("δή", "particle", "indeed, certainly, really"),
        DictionaryEntry("διά", "prep", "through (+ gen); because of (+ acc)"),
        DictionaryEntry("δίδωμι", "verb", "I give, grant, allow"),
        DictionaryEntry("δίκαιος", "adj", "just, righteous, fair"),
        DictionaryEntry("δοκέω", "verb", "I think, seem, suppose"),
        DictionaryEntry("δόξα", "noun f", "opinion; glory, reputation"),
        DictionaryEntry("δύναμαι", "verb", "I am able, can, am powerful"),
        DictionaryEntry("δύναμις", "noun f", "power, strength, ability"),
        DictionaryEntry("ἐγώ", "pron", "I (gen: ἐμοῦ/μου, dat: ἐμοί/μοι, acc: ἐμέ/με)"),
        DictionaryEntry("εἰ", "conj", "if; whether"),
        DictionaryEntry("εἰμί", "verb", "I am, exist"),
        DictionaryEntry("εἰς", "prep", "into, to, toward (+ acc)"),
        DictionaryEntry("ἐκ", "prep", "out of, from (+ gen) (ἐξ before vowels)"),
        DictionaryEntry("ἐν", "prep", "in, on, among (+ dat)"),
        DictionaryEntry("ἐπί", "prep", "on, upon (+ gen/dat/acc)"),
        DictionaryEntry("ἔργον", "noun n", "work, deed, action"),
        DictionaryEntry("ἔρχομαι", "verb", "I come, go"),
        DictionaryEntry("ἔχω", "verb", "I have, hold, possess")
    )

    fun onQueryChanged(query: String) {
        _state.value = _state.value.copy(query = query)
        search(query)
    }

    private fun search(query: String) {
        if (query.isEmpty()) {
            _state.value = _state.value.copy(results = emptyList())
            return
        }

        // Mock search: filter by headword
        val results = mockEntries.filter { entry ->
            entry.headword.contains(query, ignoreCase = true) ||
            entry.definition.contains(query, ignoreCase = true)
        }

        _state.value = _state.value.copy(results = results)
    }

    fun clearSearch() {
        _state.value = DictionaryState()
    }
}
