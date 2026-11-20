package ai.yobix;                       // keep the ai.yobix package

import org.apache.tika.parser.microsoft.OfficeParserConfig;
import org.apache.tika.parser.ocr.TesseractOCRConfig;
import org.apache.tika.parser.pdf.PDFParserConfig;

/**
 * Runs one parse so that GraalVM's native-image agent can
 * see every reflective call.
 *
 * Usage:
 *     java -jar … AgentRunner <file-to-parse>
 */
public class AgentRunner {
    public static void main(String[] args) throws Exception {
        if (args.length == 0) {
            System.err.println("Pass a file to parse");      // sanity guard
            System.exit(1);
        }

        var res = TikaNativeMain.parseFileToString(
                args[0],
                100_000,                             // char limit
                new PDFParserConfig(),
                new OfficeParserConfig(),
                new TesseractOCRConfig(),
                false);                              // plain text, not XML

        if (res.isError()) {
            System.err.println("Error: " + res.getErrorMessage());
            System.exit(1);
        }

        System.out.println("Extracted " + res.getContent().length() + " chars");
        System.out.println("Metadata: " + res.getMetadata().toString());
    }
}
