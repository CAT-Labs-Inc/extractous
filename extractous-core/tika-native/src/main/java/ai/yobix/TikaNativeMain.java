package ai.yobix;

import org.apache.commons.io.input.ReaderInputStream;
import org.apache.commons.io.IOUtils;
import org.apache.tika.Tika;
import org.apache.tika.config.TikaConfig;
import org.apache.tika.exception.TikaException;
import org.apache.tika.exception.WriteLimitReachedException;
import org.apache.tika.extractor.EmbeddedDocumentExtractor;
import org.apache.tika.io.TemporaryResources;
import org.apache.tika.io.TikaInputStream;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.metadata.TikaCoreProperties;
import org.apache.tika.metadata.HttpHeaders;
import org.apache.tika.parser.AutoDetectParser;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.parser.Parser;
import org.apache.tika.parser.microsoft.OfficeParserConfig;
import org.apache.tika.parser.ocr.TesseractOCRConfig;
import org.apache.tika.parser.pdf.PDFParserConfig;
import org.apache.tika.parser.RecursiveParserWrapper;
import org.apache.tika.sax.BodyContentHandler;
import org.apache.tika.sax.ToXMLContentHandler;
import org.apache.tika.sax.WriteOutContentHandler;
import org.apache.tika.sax.BasicContentHandlerFactory;
import org.apache.tika.sax.RecursiveParserWrapperHandler;
import org.graalvm.nativeimage.IsolateThread;
import org.graalvm.nativeimage.c.function.CEntryPoint;
import org.graalvm.nativeimage.c.type.CCharPointer;
import org.graalvm.nativeimage.c.type.CConst;
import org.graalvm.nativeimage.c.type.CTypeConversion;
import org.xml.sax.ContentHandler;
import org.xml.sax.SAXException;

import java.io.IOException;
import java.io.InputStream;
import java.io.Reader;
import java.net.MalformedURLException;
import java.net.URI;
import java.net.URISyntaxException;
import java.net.URL;
import java.nio.ByteBuffer;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;
import java.util.ArrayList;
import java.io.ByteArrayOutputStream;

public class TikaNativeMain {

    private static final Tika tika = new Tika();

    /**
     * Parses the given file and returns its type as a mime type
     *
     * @param filePath: the path of the file to be parsed
     * @return StringResult
     */
    public static StringResult detect(String filePath) {
        final Path path = Paths.get(filePath);
        final Metadata metadata = new Metadata();

        try (final InputStream stream = TikaInputStream.get(path, metadata)) {
            final String result = tika.detect(stream, metadata);
            return new StringResult(result, metadata);

        } catch (java.io.IOException e) {
            return new StringResult((byte) 1, e.getMessage());
        }
    }

    /**
     * Parses the given file and returns its content as String.
     * To avoid unpredictable excess memory use, the returned string contains only up to maxLength
     * first characters extracted from the input document.
     *
     * @param filePath:  the path of the file to be parsed
     * @param maxLength: maximum length of the returned string
     * @return StringResult
     */
    public static StringResult parseFileToString(
            String filePath,
            int maxLength,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
            // maybe replace with a single config class
    ) {
        try {
            final Path path = Paths.get(filePath);
            final Metadata metadata = new Metadata();
            final InputStream stream = TikaInputStream.get(path, metadata);

            String result = parseToStringWithConfig(
                    stream, metadata, maxLength, pdfConfig, officeConfig, tesseractConfig, asXML);
            // No need to close the stream because parseToString does so
            return new StringResult(result, metadata);
        } catch (java.io.IOException e) {
            return new StringResult((byte) 1, "Could not open file: " + e.getMessage());
        } catch (TikaException e) {
            return new StringResult((byte) 2, "Parse error occurred : " + e.getMessage());
        }
    }

    /**
     * Parses the given Url and returns its content as String
     *
     * @param urlString the url to be parsed
     * @return StringResult
     */
    public static StringResult parseUrlToString(
            String urlString,
            int maxLength,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {
        try {
            final URL url = new URI(urlString).toURL();
            final Metadata metadata = new Metadata();
            final TikaInputStream stream = TikaInputStream.get(url, metadata);

            String result = parseToStringWithConfig(
                    stream, metadata, maxLength, pdfConfig, officeConfig, tesseractConfig, asXML);
            // No need to close the stream because parseToString does so
            return new StringResult(result, metadata);

        } catch (MalformedURLException e) {
            return new StringResult((byte) 2, "Malformed URL error occurred " + e.getMessage());
        } catch (URISyntaxException e) {
            return new StringResult((byte) 2, "Malformed URI error occurred: " + e.getMessage());
        } catch (java.io.IOException e) {
            return new StringResult((byte) 1, "IO error occurred: " + e.getMessage());
        } catch (TikaException e) {
            return new StringResult((byte) 2, "Parse error occurred : " + e.getMessage());
        }
    }

    /**
     * Parses the given array of bytes and return its content as String.
     *
     * @param data an array of bytes
     * @return StringResult
     */
    public static StringResult parseBytesToString(
            ByteBuffer data,
            int maxLength,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {
        final Metadata metadata = new Metadata();
        final ByteBufferInputStream inStream = new ByteBufferInputStream(data);
        final TikaInputStream stream = TikaInputStream.get(inStream, new TemporaryResources(), metadata);

        try {
            String result = parseToStringWithConfig(
                    stream, metadata, maxLength, pdfConfig, officeConfig, tesseractConfig, asXML);
            // No need to close the stream because parseToString does so
            return new StringResult(result, metadata);
        } catch (java.io.IOException e) {
            return new StringResult((byte) 1, "IO error occurred: " + e.getMessage());
        } catch (TikaException e) {
            return new StringResult((byte) 2, "Parse error occurred : " + e.getMessage());
        }
    }

    private static String parseToStringWithConfig(
            InputStream stream,
            Metadata metadata,
            int maxLength,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) throws IOException, TikaException {
        ContentHandler handler;
        ContentHandler handlerForParser;
        if (asXML) {
            handler = new WriteOutContentHandler(new ToXMLContentHandler(), maxLength);
            handlerForParser = handler;
        } else {
            handler = new WriteOutContentHandler(maxLength);
            handlerForParser = new BodyContentHandler(handler);
        }

        try {
            final ParseContext parsecontext = new ParseContext();
            final Parser parser = TikaShared.parser();

            parsecontext.set(Parser.class, parser);
            parsecontext.set(PDFParserConfig.class, pdfConfig);
            parsecontext.set(OfficeParserConfig.class, officeConfig);
            parsecontext.set(TesseractOCRConfig.class, tesseractConfig);

            parser.parse(stream, handlerForParser, metadata, parsecontext);
        } catch (SAXException e) {
            if (!WriteLimitReachedException.isWriteLimitReached(e)) {
                // This should never happen with BodyContentHandler...
                throw new TikaException("Unexpected SAX processing failure", e);
            }
        } finally {
            stream.close();
        }
        return handler.toString();
    }


    /**
     * Parses the given file and returns its content as Reader. The reader can be used
     * to read chunks and must be closed when reading is finished
     *
     * @param filePath the path of the file
     * @return ReaderResult
     */
    public static ReaderResult parseFile(
            String filePath,
            String charsetName,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {
        try {
//            System.out.println("pdfConfig.isExtractInlineImages = " + pdfConfig.isExtractInlineImages());
//            System.out.println("pdfConfig.isExtractMarkedContent = " + pdfConfig.isExtractMarkedContent());
//            System.out.println("pdfConfig.getOcrStrategy = " + pdfConfig.getOcrStrategy());
//            System.out.println("officeConfig.isIncludeHeadersAndFooters = " + officeConfig.isIncludeHeadersAndFooters());
//            System.out.println("officeConfig.isIncludeShapeBasedContent = " + officeConfig.isIncludeShapeBasedContent());
//            System.out.println("ocrConfig.getTimeoutSeconds = " + tesseractConfig.getTimeoutSeconds());
//            System.out.println("ocrConfig.language = " + tesseractConfig.getLanguage());

            final Path path = Paths.get(filePath);
            final Metadata metadata = new Metadata();
            final TikaInputStream stream = TikaInputStream.get(path, metadata);

            return parse(stream, metadata, charsetName, pdfConfig, officeConfig, tesseractConfig, asXML);

        } catch (java.io.IOException e) {
            return new ReaderResult((byte) 1, "Could not open file: " + e.getMessage());
        }
    }

    /**
     * Parses the given Url and returns its content as Reader. The reader can be used
     * to read chunks and must be closed when reading is finished
     *
     * @param urlString the url to be parsed
     * @return ReaderResult
     */
    public static ReaderResult parseUrl(
            String urlString,
            String charsetName,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {
        try {
            final URL url = new URI(urlString).toURL();
            final Metadata metadata = new Metadata();
            final TikaInputStream stream = TikaInputStream.get(url, metadata);

            return parse(stream, metadata, charsetName, pdfConfig, officeConfig, tesseractConfig, asXML);

        } catch (MalformedURLException e) {
            return new ReaderResult((byte) 2, "Malformed URL error occurred " + e.getMessage());
        } catch (URISyntaxException e) {
            return new ReaderResult((byte) 3, "Malformed URI error occurred: " + e.getMessage());
        } catch (java.io.IOException e) {
            return new ReaderResult((byte) 1, "IO error occurred: " + e.getMessage());
        }
    }

    /**
     * Parses the given array of bytes and return its content as Reader. The reader can be used
     * to read chunks and must be closed when reading is finished
     *
     * @param data an array of bytes
     * @return ReaderResult
     */
    public static ReaderResult parseBytes(
            ByteBuffer data,
            String charsetName,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {


        final Metadata metadata = new Metadata();
        final ByteBufferInputStream inStream = new ByteBufferInputStream(data);
        final TikaInputStream stream = TikaInputStream.get(inStream, new TemporaryResources(), metadata);

        return parse(stream, metadata, charsetName, pdfConfig, officeConfig, tesseractConfig, asXML);
    }

    private static ReaderResult parse(
            TikaInputStream inputStream,
            Metadata metadata,
            String charsetName,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            boolean asXML
    ) {
        try {

            final ParseContext parsecontext = new ParseContext();
            final Parser parser = TikaShared.parser();
            final Charset charset = Charset.forName(charsetName, StandardCharsets.UTF_8);

            parsecontext.set(Parser.class, parser);
            parsecontext.set(PDFParserConfig.class, pdfConfig);
            parsecontext.set(OfficeParserConfig.class, officeConfig);
            parsecontext.set(TesseractOCRConfig.class, tesseractConfig);

            //final Reader reader = new org.apache.tika.parser.ParsingReader(parser, inputStream, metadata, parsecontext);
            final Reader reader = new ParsingReader(parser, inputStream, metadata, parsecontext, asXML, charset.name());

            // Convert Reader which works with chars to ReaderInputStream which works with bytes
            ReaderInputStream readerInputStream = ReaderInputStream.builder()
                    .setReader(reader)
                    .setCharset(charset)
                    .get();

            return new ReaderResult(readerInputStream, metadata);

        } catch (java.io.IOException e) {
            return new ReaderResult((byte) 1, "IO error occurred: " + e.getMessage());
        }

    }

    /**
     * This is the main entry point of the native image build. @CEntryPoint is used
     * because we do not want to build an executable with a main method. The gradle nativeImagePlugin
     * expects either a main method or @CEntryPoint
     * This uses the C Api isolate, which is can only work with primitive return types unlike the JNI invocation
     * interface.
     */
    @CEntryPoint(name = "c_parse_to_string")
    private static CCharPointer cParseToString(IsolateThread thread, @CConst CCharPointer cFilePath) {
        final String filePath = CTypeConversion.toJavaString(cFilePath);

        final Path path = Paths.get(filePath);
        try {
            final String content = tika.parseToString(path);
            try (CTypeConversion.CCharPointerHolder holder = CTypeConversion.toCString(content)) {
                return holder.get();
            }

        } catch (java.io.IOException | TikaException e) {
            throw new RuntimeException(e);
        }
    }

    /**
     * Extracts all embedded documents/resources from a file (images, attachments, etc.)
     * This is similar to Tika Server's /unpack/all endpoint
     * 
     * @param filePath the path of the file to extract embedded content from
     * @return EmbeddedExtractResult containing all embedded documents
     */
    public static EmbeddedExtractResult extractEmbedded(
            String filePath,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig
    ) {
        try {
            final Path path = Paths.get(filePath);
            final Metadata metadata = new Metadata();
            final TikaInputStream stream = TikaInputStream.get(path, metadata);
            
            return extractEmbeddedFromStream(stream, metadata, pdfConfig, officeConfig, tesseractConfig);
        } catch (IOException e) {
            return new EmbeddedExtractResult((byte) 1, "Could not open file: " + e.getMessage());
        }
    }

    /**
     * Extracts all embedded documents/resources from a byte buffer
     * 
     * @param data the byte buffer containing the document
     * @return EmbeddedExtractResult containing all embedded documents
     */
    public static EmbeddedExtractResult extractEmbeddedFromBytes(
            ByteBuffer data,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig
    ) {
        final Metadata metadata = new Metadata();
        final ByteBufferInputStream inStream = new ByteBufferInputStream(data);
        final TikaInputStream stream = TikaInputStream.get(inStream, new TemporaryResources(), metadata);
        
        return extractEmbeddedFromStream(stream, metadata, pdfConfig, officeConfig, tesseractConfig);
    }

    /**
     * Optimized extraction method that packs all data into a single ByteBuffer
     */
    public static OptimizedEmbeddedExtractor.OptimizedResult extractEmbeddedOptimized(
            String filePath,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig
    ) {
        // Use the OptimizedEmbeddedExtractor with a reasonable document limit
        return OptimizedEmbeddedExtractor.extractEmbeddedOptimized(
            filePath, 
            pdfConfig, 
            officeConfig, 
            tesseractConfig,
            1000 // Maximum 1000 embedded documents
        );
    }
    
    /**
     * Shared helper method to extract embedded documents from a stream
     */
    private static EmbeddedExtractResult extractEmbeddedFromStream(
            TikaInputStream stream,
            Metadata metadata,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig
    ) {
        final List<EmbeddedExtractResult.EmbeddedDocument> embeddedDocuments = new ArrayList<>();
        
        try {
            final ParseContext parseContext = new ParseContext();
            final Parser baseParser = TikaShared.parser();

            // Configure parse context
            parseContext.set(Parser.class, baseParser);
            parseContext.set(PDFParserConfig.class, pdfConfig);
            parseContext.set(OfficeParserConfig.class, officeConfig);
            parseContext.set(TesseractOCRConfig.class, tesseractConfig);
            
            // Custom embedded document extractor
            EmbeddedDocumentExtractor embeddedExtractor = new EmbeddedDocumentExtractor() {
                @Override
                public boolean shouldParseEmbedded(Metadata metadata) {
                    return true; // Extract all embedded documents
                }
                
                @Override
                public void parseEmbedded(
                        InputStream inputStream,
                        ContentHandler contentHandler,
                        Metadata embeddedMetadata,
                        boolean outputHtml) throws SAXException, IOException {
                    
                    // Get metadata
                    String resourceName = embeddedMetadata.get(TikaCoreProperties.RESOURCE_NAME_KEY);
                    String contentType = embeddedMetadata.get(HttpHeaders.CONTENT_TYPE);
                    String embeddedRelationshipId = embeddedMetadata.get(TikaCoreProperties.EMBEDDED_RELATIONSHIP_ID);
                    
                    // Default values if metadata is missing
                    if (resourceName == null) {
                        resourceName = "embedded_" + embeddedDocuments.size();
                    }
                    if (contentType == null) {
                        contentType = "application/octet-stream";
                    }
                    
                    // Read content into byte array
                    ByteArrayOutputStream baos = new ByteArrayOutputStream();
                    IOUtils.copy(inputStream, baos);
                    byte[] content = baos.toByteArray();
                    
                    // Create embedded document
                    embeddedDocuments.add(new EmbeddedExtractResult.EmbeddedDocument(
                        resourceName,
                        contentType,
                        content,
                        embeddedRelationshipId
                    ));
                }
            };
            
            parseContext.set(EmbeddedDocumentExtractor.class, embeddedExtractor);
            
            // Use RecursiveParserWrapper to extract all embedded content
            RecursiveParserWrapper wrapper = new RecursiveParserWrapper(baseParser);
            BasicContentHandlerFactory factory = new BasicContentHandlerFactory(
                BasicContentHandlerFactory.HANDLER_TYPE.TEXT, -1);
            RecursiveParserWrapperHandler handler = new RecursiveParserWrapperHandler(factory);
            
            wrapper.parse(stream, handler, metadata, parseContext);
            
            return new EmbeddedExtractResult(embeddedDocuments);
            
        } catch (IOException e) {
            return new EmbeddedExtractResult((byte) 1, "IO error occurred: " + e.getMessage());
        } catch (SAXException e) {
            return new EmbeddedExtractResult((byte) 2, "SAX error occurred: " + e.getMessage());
        } catch (TikaException e) {
            return new EmbeddedExtractResult((byte) 3, "Tika error occurred: " + e.getMessage());
        } catch (Exception e) {
            return new EmbeddedExtractResult((byte) 99, "Unexpected error: " + e.getClass().getName() + ": " + e.getMessage());
        } finally {
            try {
                stream.close();
            } catch (IOException ignored) {
            }
        }
    }

}
