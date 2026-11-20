package ai.yobix;

import java.util.List;
import java.util.ArrayList;

/**
 * Result class for embedded document extraction operations
 */
public class EmbeddedExtractResult {
    private final byte errorCode;
    private final String errorMessage;
    private final List<EmbeddedDocument> embeddedDocuments;

    /**
     * Success result constructor
     */
    public EmbeddedExtractResult(List<EmbeddedDocument> embeddedDocuments) {
        this.errorCode = 0;
        this.errorMessage = null;
        this.embeddedDocuments = embeddedDocuments;
    }

    /**
     * Error result constructor
     */
    public EmbeddedExtractResult(byte errorCode, String errorMessage) {
        this.errorCode = errorCode;
        this.errorMessage = errorMessage;
        this.embeddedDocuments = new ArrayList<>();
    }

    public boolean hasError() {
        return errorCode != 0;
    }

    public byte getErrorCode() {
        return errorCode;
    }

    public String getErrorMessage() {
        return errorMessage;
    }

    public List<EmbeddedDocument> getEmbeddedDocuments() {
        return embeddedDocuments;
    }

    /**
     * Represents a single embedded document/resource
     */
    public static class EmbeddedDocument {
        private final String resourceName;
        private final String contentType;
        private final byte[] content;
        private final String embeddedRelationshipId;

        public EmbeddedDocument(String resourceName, String contentType, byte[] content, String embeddedRelationshipId) {
            this.resourceName = resourceName;
            this.contentType = contentType;
            this.content = content;
            this.embeddedRelationshipId = embeddedRelationshipId;
        }

        public String getResourceName() {
            return resourceName;
        }

        public String getContentType() {
            return contentType;
        }

        public byte[] getContent() {
            return content;
        }

        public String getEmbeddedRelationshipId() {
            return embeddedRelationshipId;
        }
    }
}