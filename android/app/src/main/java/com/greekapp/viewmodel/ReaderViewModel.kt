package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class WordAnalysis(
    val word: String,
    val lemma: String,
    val pos: String,
    val definition: String,
    val parseInfo: String? = null
)

data class ReaderState(
    val text: String = """Ὁ Δικαιόπολις ἐν τοῖς ἀγροῖς πονεῖ. γεωργός γάρ ἐστι. καὶ ἰσχυρὸς μὲν ὁ γεωργός, μακρὸς δὲ ὁ πόνος. ὁ οὖν Δικαιόπολις πονεῖ ἐν τοῖς ἀγροῖς καὶ μάλα κάμνει. ἄρα οὐ πονεῖ; πονεῖ μὲν ἀλλὰ κάμνει.""",
    val selectedWord: WordAnalysis? = null,
    val showAnalysis: Boolean = false
)

class ReaderViewModel : ViewModel() {
    private val _state = MutableStateFlow(ReaderState())
    val state: StateFlow<ReaderState> = _state.asStateFlow()

    // Mock dictionary data
    private val mockDictionary = mapOf(
        "ὁ" to WordAnalysis("ὁ", "ὁ", "article", "the (masc nom sg)", "article m nom sg"),
        "δικαιόπολις" to WordAnalysis("Δικαιόπολις", "Δικαιόπολις", "noun", "Dicaeopolis (name)", "proper noun m nom sg"),
        "ἐν" to WordAnalysis("ἐν", "ἐν", "prep", "in, on, among (+ dat)", "preposition"),
        "τοῖς" to WordAnalysis("τοῖς", "ὁ", "article", "the (masc/neut dat pl)", "article m/n dat pl"),
        "ἀγροῖς" to WordAnalysis("ἀγροῖς", "ὁ ἀγρός", "noun", "field, farm", "noun m dat pl"),
        "πονεῖ" to WordAnalysis("πονεῖ", "πονέω", "verb", "he/she/it works, toils", "verb 3s pres act ind"),
        "γεωργός" to WordAnalysis("γεωργός", "ὁ γεωργός", "noun", "farmer", "noun m nom sg"),
        "γάρ" to WordAnalysis("γάρ", "γάρ", "particle", "for, because", "explanatory particle"),
        "ἐστι" to WordAnalysis("ἐστι", "εἰμί", "verb", "he/she/it is", "verb 3s pres act ind"),
        "καί" to WordAnalysis("καί", "καί", "conj", "and, also, even", "conjunction"),
        "ἰσχυρός" to WordAnalysis("ἰσχυρός", "ἰσχυρός", "adj", "strong, mighty", "adjective m nom sg"),
        "μέν" to WordAnalysis("μέν", "μέν", "particle", "on the one hand (balanced with δέ)", "particle"),
        "μακρός" to WordAnalysis("μακρός", "μακρός", "adj", "long, large", "adjective m nom sg"),
        "δέ" to WordAnalysis("δέ", "δέ", "particle", "but, and (postpositive)", "particle"),
        "πόνος" to WordAnalysis("πόνος", "ὁ πόνος", "noun", "work, toil, labor", "noun m nom sg"),
        "οὖν" to WordAnalysis("οὖν", "οὖν", "particle", "therefore, then (postpositive)", "particle"),
        "μάλα" to WordAnalysis("μάλα", "μάλα", "adv", "very, very much", "adverb"),
        "κάμνει" to WordAnalysis("κάμνει", "κάμνω", "verb", "he/she/it grows weary, is tired", "verb 3s pres act ind"),
        "ἄρα" to WordAnalysis("ἄρα", "ἄρα", "particle", "then, therefore (marks a question)", "inferential particle"),
        "οὐ" to WordAnalysis("οὐ", "οὐ", "adv", "not", "negative adverb"),
        "ἀλλά" to WordAnalysis("ἀλλά", "ἀλλά", "conj", "but, however", "strong adversative conjunction")
    )

    fun onWordTapped(word: String) {
        val normalized = word.toLowerCase().trim(' ', '.', ',', ';', '·', '?', '!')
        val analysis = mockDictionary[normalized] ?: WordAnalysis(
            word = word,
            lemma = "Unknown",
            pos = "unknown",
            definition = "Word not in dictionary (mock data only)"
        )

        _state.value = _state.value.copy(
            selectedWord = analysis,
            showAnalysis = true
        )
    }

    fun dismissAnalysis() {
        _state.value = _state.value.copy(
            showAnalysis = false,
            selectedWord = null
        )
    }

    fun addToSrs() {
        // In real app: GreekCore.addWordToSrs(selectedWord.lemma)
        dismissAnalysis()
    }
}
