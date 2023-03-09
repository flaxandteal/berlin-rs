# berlin

A Rust crate to identify locations and tag them with UN-LOCODEs and
ISO-3166-2 subdivisions.


### Description

Berlin is a location search engine which  works on an in-memory collection of
all UN Locodes, subdivisions and states (countries). Here are the main
architectural highlights: On startup Berlin does a basic linguistic analysis of
the locations: split names into words, remove diacritics, transliterate
non-ASCII symbols to ASCII. For example,  this allows us to find  “Las Vegas”
when searching for “vegas”.  It employs string interning in order to both
optimise memory usage and allow direct lookups for exact matches. If we can
resolve (parts of) the search term to an existing interned string, it means
that we have a location with this name in the database.

When the user submits the search term, Berlin first does a preliminary analysis
of the search term: 1) split into words and pairs of words 2) try to identify
the former as existing locations (can be resolved to existing interned strings)
and tag them as “exact matches”. This creates many search terms from the
original phrase.  Pre-filtering step. Here we do three things 1) resolve exact
matches by direct lookup in the names and codes tables 2) do a prefix search
via a finite-state transducer 3) do a fuzzy search via a Levenshtein distance
enabled finite-state transducer.  The pre-filtered results are passed through a
string-similarity evaluation algorithm and sorted by score. The results below a
threshold are truncated.  A graph is built from the locations found during the
previous  step in order to link them together hierarchically if possible. This
further boosts some locations. For example, if the user searches for “new york
UK” it will boost the location in Lincolnshire and it will show up higher than
New York city in the USA.  It is also possible to request search only in a
specific country (which is enabled by default for the UK)

Berlin is able to find locations with a high degree of semantic accuracy. Speed
is roughly equal to 10-15 ms per every non-matching word (or typo) + 1 ms for
every exact match. A complex query of 8 words usually takes less than 100 ms
and all of the realistic queries in our test suite take less than 50 ms, while
the median is under 30 ms. Short queries containing  an exact match (case
insensitive) are faster than 10 ms.

The architecture would allow to easily implement as-you-type search suggestions
in under 10 milliseconds if deemed desirable.


### License

Prepared by Flax & Teal Limited for ONS Alpha and ONS Beta projects.
Copyright © 2022-2023, ONS Digital (https://www.ons.gov.uk)

Released under MIT license, see [LICENSE](LICENSE.md) for details.
