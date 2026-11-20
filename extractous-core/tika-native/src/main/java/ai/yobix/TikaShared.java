package ai.yobix;

import org.apache.tika.config.TikaConfig;
import org.apache.tika.parser.AutoDetectParser;
import org.apache.tika.parser.Parser;

/**
 * Process-wide shared {@link TikaConfig} and {@link Parser}.
 *
 * <p>{@code TikaConfig.getDefaultConfig()} is not a cached singleton — it is
 * literally {@code new TikaConfig()}, which runs full {@code ServiceLoader}
 * discovery and instantiates every parser and detector on the classpath, then
 * builds the CompositeParser media-type map. Measured at 34-38 ms warm, and it
 * was previously paid on every extraction call (twice per document, since
 * catscan makes both a text pass and an embedded pass).
 *
 * <p>Both types are documented thread-safe and are shared across threads by
 * Tika Server, so a single instance is safe for the Rayon-parallel callers.
 * Per-call configurability is unaffected: {@code PDFParserConfig},
 * {@code OfficeParserConfig} and {@code TesseractOCRConfig} continue to be
 * supplied per call through the {@code ParseContext}.
 *
 * <p>The instances live in a nested holder class so that initialization happens
 * on first access rather than when {@code TikaShared} itself is initialized.
 * That keeps the heavy {@code ServiceLoader} work out of native-image build
 * time even if this class is reachable from an {@code --initialize-at-build-time}
 * graph.
 */
final class TikaShared {

    private TikaShared() {
    }

    private static final class Holder {
        static final TikaConfig CONFIG = TikaConfig.getDefaultConfig();
        static final Parser PARSER = new AutoDetectParser(CONFIG);
    }

    /** The shared default configuration. */
    static TikaConfig config() {
        return Holder.CONFIG;
    }

    /** The shared auto-detecting parser built from {@link #config()}. */
    static Parser parser() {
        return Holder.PARSER;
    }
}
