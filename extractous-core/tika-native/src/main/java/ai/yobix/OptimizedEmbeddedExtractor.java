package ai.yobix;

import org.apache.tika.config.TikaConfig;
import org.apache.tika.exception.TikaException;
import org.apache.tika.extractor.EmbeddedDocumentExtractor;
import org.apache.tika.io.TikaInputStream;
import org.apache.tika.io.TemporaryResources;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.metadata.TikaCoreProperties;
import org.apache.tika.parser.AutoDetectParser;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.parser.Parser;
import org.apache.tika.parser.RecursiveParserWrapper;
import org.apache.tika.parser.microsoft.OfficeParserConfig;
import org.apache.tika.parser.ocr.TesseractOCRConfig;
import org.apache.tika.parser.pdf.PDFParserConfig;
import org.apache.tika.sax.BasicContentHandlerFactory;
import org.apache.tika.sax.RecursiveParserWrapperHandler;
import org.xml.sax.ContentHandler;
import org.xml.sax.SAXException;

import java.io.*;
import java.nio.ByteBuffer;
import java.nio.channels.WritableByteChannel;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

/**
 * Optimized embedded document extractor that minimizes JNI overhead
 */
public class OptimizedEmbeddedExtractor {
    
    /**
     * Optimized result that transfers all data in a single ByteBuffer to minimize JNI calls
     */
    public static class OptimizedResult {
        private final byte errorCode;
        private final String errorMessage;
        private final ByteBuffer packedData;
        private final int documentCount;
        
        // Success constructor
        public OptimizedResult(ByteBuffer packedData, int documentCount) {
            this.errorCode = 0;
            this.errorMessage = null;
            this.packedData = packedData;
            this.documentCount = documentCount;
        }
        
        // Error constructor
        public OptimizedResult(byte errorCode, String errorMessage) {
            this.errorCode = errorCode;
            this.errorMessage = errorMessage;
            this.packedData = null;
            this.documentCount = 0;
        }
        
        // Getters for JNI access
        public byte getErrorCode() { return errorCode; }
        public String getErrorMessage() { return errorMessage; }
        public ByteBuffer getPackedData() { return packedData; }
        public int getDocumentCount() { return documentCount; }
    }
    
    /**
     * Extract embedded documents and pack them into a single ByteBuffer
     * Format: [count][doc1_size][doc1_data][doc2_size][doc2_data]...
     * Where each doc_data contains: [name_len][name][type_len][type][content_len][content]
     */
    public static OptimizedResult extractEmbeddedOptimized(
            String filePath,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            int maxDocuments  // Add limit to prevent memory issues
    ) {
        try {
            final Path path = Paths.get(filePath);
            final Metadata metadata = new Metadata();
            final InputStream stream = TikaInputStream.get(path, metadata);
            
            return extractEmbeddedFromStreamOptimized(stream, metadata, pdfConfig, officeConfig, tesseractConfig, maxDocuments);
        } catch (IOException e) {
            return new OptimizedResult((byte) 1, "Could not open file: " + e.getMessage());
        }
    }
    
    private static OptimizedResult extractEmbeddedFromStreamOptimized(
            InputStream stream,
            Metadata metadata,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            int maxDocuments
    ) {
        // Use ByteArrayOutputStream to collect all data
        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        DataOutputStream dos = new DataOutputStream(baos);
        final int[] documentCount = {0};
        
        try {
            final ParseContext parseContext = new ParseContext();
            final Parser baseParser = TikaShared.parser();

            // Configure parse context
            parseContext.set(Parser.class, baseParser);
            parseContext.set(PDFParserConfig.class, pdfConfig);
            parseContext.set(OfficeParserConfig.class, officeConfig);
            parseContext.set(TesseractOCRConfig.class, tesseractConfig);
            
            // Write placeholder for document count (will update later)
            dos.writeInt(0);
            
            // Custom embedded document extractor that writes directly to buffer
            EmbeddedDocumentExtractor embeddedExtractor = new EmbeddedDocumentExtractor() {
                @Override
                public boolean shouldParseEmbedded(Metadata metadata) {
                    // Stop if we've reached the limit
                    return maxDocuments <= 0 || documentCount[0] < maxDocuments;
                }
                
                @Override
                public void parseEmbedded(
                        InputStream inputStream,
                        ContentHandler contentHandler,
                        Metadata embeddedMetadata,
                        boolean outputHtml) throws SAXException, IOException {
                    
                    // Skip if we've reached the limit
                    if (maxDocuments > 0 && documentCount[0] >= maxDocuments) {
                        return;
                    }
                    
                    String resourceName = embeddedMetadata.get(TikaCoreProperties.RESOURCE_NAME_KEY);
                    String contentType = embeddedMetadata.get(org.apache.tika.metadata.HttpHeaders.CONTENT_TYPE);
                    String embeddedRelationshipId = embeddedMetadata.get(TikaCoreProperties.EMBEDDED_RELATIONSHIP_ID);
                    
                    // Default values
                    if (resourceName == null) resourceName = "embedded_" + documentCount[0];
                    if (contentType == null) contentType = "application/octet-stream";
                    if (embeddedRelationshipId == null) embeddedRelationshipId = "";
                    
                    // Read content efficiently
                    byte[] contentBytes = readStream(inputStream);
                    
                    // Write to data output stream
                    writeString(dos, resourceName);
                    writeString(dos, contentType);
                    writeString(dos, embeddedRelationshipId);
                    dos.writeInt(contentBytes.length);
                    dos.write(contentBytes);
                    
                    documentCount[0]++;
                }
                
                private byte[] readStream(InputStream is) throws IOException {
                    ByteArrayOutputStream buffer = new ByteArrayOutputStream();
                    byte[] data = new byte[16384]; // Larger buffer for better performance
                    int nRead;
                    while ((nRead = is.read(data, 0, data.length)) != -1) {
                        buffer.write(data, 0, nRead);
                    }
                    return buffer.toByteArray();
                }
                
                private void writeString(DataOutputStream dos, String str) throws IOException {
                    byte[] bytes = str.getBytes("UTF-8");
                    dos.writeInt(bytes.length);
                    dos.write(bytes);
                }
            };
            
            parseContext.set(EmbeddedDocumentExtractor.class, embeddedExtractor);
            
            // Use RecursiveParserWrapper
            RecursiveParserWrapper wrapper = new RecursiveParserWrapper(baseParser);
            BasicContentHandlerFactory factory = new BasicContentHandlerFactory(
                BasicContentHandlerFactory.HANDLER_TYPE.TEXT, -1);
            RecursiveParserWrapperHandler handler = new RecursiveParserWrapperHandler(factory);
            
            wrapper.parse(stream, handler, metadata, parseContext);
            
            // Update document count at the beginning of the buffer
            dos.close();
            byte[] data = baos.toByteArray();
            
            // Create direct ByteBuffer and update count
            ByteBuffer buffer = ByteBuffer.allocateDirect(data.length);
            buffer.put(data);
            buffer.putInt(0, documentCount[0]); // Update count at position 0
            buffer.flip();
            
            return new OptimizedResult(buffer, documentCount[0]);
            
        } catch (IOException e) {
            return new OptimizedResult((byte) 1, "IO error occurred: " + e.getMessage());
        } catch (SAXException e) {
            return new OptimizedResult((byte) 2, "SAX error occurred: " + e.getMessage());
        } catch (TikaException e) {
            return new OptimizedResult((byte) 3, "Tika error occurred: " + e.getMessage());
        } catch (Exception e) {
            return new OptimizedResult((byte) 99, "Unexpected error: " + e.getClass().getName() + ": " + e.getMessage());
        } finally {
            try {
                stream.close();
            } catch (IOException ignored) {
            }
        }
    }
    
    /**
     * Streaming version that processes documents in batches
     */
    public static OptimizedResult extractEmbeddedBatch(
            String filePath,
            PDFParserConfig pdfConfig,
            OfficeParserConfig officeConfig,
            TesseractOCRConfig tesseractConfig,
            int offset,
            int limit
    ) {
        // Implementation for batch extraction
        // This would require maintaining state between calls
        // For now, we'll use the optimized version with limits
        return extractEmbeddedOptimized(filePath, pdfConfig, officeConfig, tesseractConfig, limit);
    }
}