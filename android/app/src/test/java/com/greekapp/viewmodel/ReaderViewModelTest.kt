package com.greekapp.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.uniffi_bindings.AnalysisView

class ReaderViewModelTest {
    private fun analysis(
        lemma: String,
        definition: String,
        pos: String,
        parse: String,
        lemmaId: Long = 1L
    ) = AnalysisView(
        lemma = lemma,
        lemmaId = lemmaId,
        postag = "raw-tag",
        postagHuman = parse,
        definition = definition,
        confidence = 1f,
        pos = pos
    )

    @Test
    fun candidates_preserveRankedLemmasAndEveryParse() {
        val candidates = wordCandidates(
            listOf(
                analysis("ὁ", "the", "article", "article singular feminine accusative"),
                analysis("ἄτη", "ruin", "noun", "noun singular feminine accusative"),
                analysis("ὁ", "the", "article", "article singular masculine accusative")
            )
        )

        assertEquals(listOf("ὁ", "ἄτη"), candidates.map { it.lemma })
        assertEquals("the", candidates[0].definition)
        assertEquals("article", candidates[0].pos)
        assertEquals(
            listOf(
                "article singular feminine accusative",
                "article singular masculine accusative"
            ),
            candidates[0].parseSummaries
        )
        assertEquals("noun", candidates[1].pos)
        assertEquals("ruin", candidates[1].definition)
    }

    @Test
    fun candidates_keepDistinctDefinitionsAndUseHonestBlankGlossFallback() {
        val candidates = wordCandidates(
            listOf(
                analysis("λέγω", "say", "verb", "verb present active"),
                analysis("λέγω", "gather", "verb", "verb aorist active"),
                analysis("ἄν", "", "particle", "particle")
            )
        )

        assertEquals("say\ngather", candidates[0].definition)
        assertEquals(2, candidates[0].parseSummaries.size)
        assertEquals("No definition available", candidates[1].definition)
    }
}
