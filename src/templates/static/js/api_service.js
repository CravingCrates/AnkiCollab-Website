/**
 * Service for making API calls related to commits, notes, fields, tags, etc.
 */
window.ApiService = (function() {

    // --- Constants ---
    const API_BASE = ''; // Adjust if you have a base path like /api/v1
    let lastResponseMeta = null;

    // --- Private Helper Functions ---

    /**
     * Gets context (type and ID) from a DOM element or its ancestors.
     * @param {HTMLElement} element - The starting element.
     * @returns {{type: string|null, id: string|null}} - The context type and ID.
     */
    function getContext(element) {
        const $contextElement = $(element).closest('[data-context-type]');
        if (!$contextElement.length) {
            console.error("Could not find context element for:", element);
            return { type: null, id: null };
        }
        const contextType = $contextElement.data('context-type'); // 'note'
        const contextId = $contextElement.attr('id'); // The note ID
        return { type: contextType, id: contextId };
    }

    /**
     * Generic function to make API calls using fetch.
     * @param {string} endpoint - The API endpoint (e.g., /AcceptTag/123).
     * @param {string} [method='GET'] - HTTP method.
     * @param {object|null} [data=null] - Data payload for POST/PUT requests.
     * @returns {Promise<object|string|null>} - Resolves with parsed JSON, text, or null/success status. Rejects on error.
     */
    async function apiCall(endpoint, method = 'GET', data = null) {
        const url = `${API_BASE}${endpoint}`;
        const options = {
            method: method,
            headers: {},
        };

        if (data && (method === 'POST' || method === 'PUT' || method === 'PATCH')) {
            options.headers['Content-Type'] = 'application/json';
            options.body = JSON.stringify(data);
        }

        lastResponseMeta = null;

        try {
            const response = await fetch(url, options);

            lastResponseMeta = {
                status: response.status,
                redirected: response.redirected,
                url: response.url
            };

            if (!response.ok) {
                let errorMsg = `HTTP error! Status: ${response.status}`;
                let errorData = null;
                try {
                    // Try to parse error response, but don't fail if it's not JSON
                    errorData = await response.json();
                    errorMsg = errorData.message || errorData.error || errorMsg;
                } catch (e) { /* Ignore parsing error, use status text */
                    errorMsg = response.statusText || errorMsg;
                }
                // Throw an error object that includes potential details
                const error = new Error(errorMsg);
                error.status = response.status;
                error.data = errorData;
                throw error;
            }

            // Handle different success scenarios
            if (response.status === 204 || response.headers.get('Content-Length') === '0') {
                return null; // No content, resolve with null
            }

            // Try to parse as JSON, fall back to text if needed
            const contentType = response.headers.get('content-type');
            if (contentType && contentType.includes('application/json')) {
                return await response.json();
            } else {
                return await response.text(); // Return text for non-JSON (like diff HTML)
            }

        } catch (error) {
            console.error(`API call failed for ${method} ${url}:`, error);
            // Re-throw the error so the caller can handle it (e.g., update UI)
            throw error;
        }
    }

    // --- Public API Methods ---

    // -- Tag Actions --
    function acceptTag(tagId) {
        return apiCall(`/AcceptTag/${tagId}`, 'GET');
    }

    function denyTag(tagId) {
        return apiCall(`/DenyTag/${tagId}`, 'GET');
    }

    // -- Move Actions --
    function acceptMove(moveReqId) {
        return apiCall(`/AcceptNoteMove/${moveReqId}`, 'GET');
    }

    function denyMove(moveReqId) {
        return apiCall(`/DenyNoteMove/${moveReqId}`, 'GET');
    }

    // -- Field Actions --
    function acceptField(fieldId) {
        // Returns { success: true, new_content: "..." } or similar on success
        return apiCall(`/AcceptField/${fieldId}`, 'GET');
    }

    function denyField(fieldId) {
        // Returns { success: true, original_content: "..." } or similar on success
        return apiCall(`/DenyField/${fieldId}`, 'GET');
    }

    // -- Note Actions --
    function acceptNote(noteId) {
        return apiCall(`/AcceptNote/${noteId}`, 'GET');
    }

    function deleteNote(noteId) {
        return apiCall(`/DeleteNote/${noteId}`, 'GET');
    }

    function acceptNoteRemoval(noteId) {
        return apiCall(`/AcceptNoteRemoval/${noteId}`, 'GET');
    }

    function denyNoteRemoval(noteId) {
        return apiCall(`/DenyNoteRemoval/${noteId}`, 'GET');
    }

    // -- Field Suggestion Update --
    function updateFieldSuggestion(fieldId, content) {
        return apiCall(`/UpdateFieldSuggestion`, 'POST', { field_id: fieldId, content: content });
    }

    // -- Edit All Fields API --
    /**
     * Gets all fields for a note to populate the edit panel.
     * @param {number|string} noteId - The note ID
     * @param {number|string} commitId - The commit ID to scope suggestions to
     * @returns {Promise<object>} - Resolves with AllFieldsForEditResponse
     */
    function getAllFieldsForEdit(noteId, commitId) {
        return apiCall(`/GetAllFieldsForEdit/${noteId}/${commitId}`, 'GET');
    }

    /**
     * Batch creates or updates field suggestions for a note.
     * @param {number|string} noteId - The note ID
     * @param {number|string} commitId - The commit ID to associate with
     * @param {Array<{position: number, content: string}>} fields - Array of field updates
     * @returns {Promise<object>} - Resolves with BatchFieldSuggestionResponse
     */
    function batchUpdateFieldSuggestions(noteId, commitId, fields) {
        return apiCall('/BatchUpdateFieldSuggestions', 'POST', {
            note_id: parseInt(noteId, 10),
            commit_id: parseInt(commitId, 10),
            fields: fields
        });
    }

    /**
     * Bulk approve or deny selected notes within a commit.
     * @param {number|string} commitId - The commit ID
     * @param {Array<number>} noteIds - Array of note IDs to process
     * @param {string} action - Either 'approve' or 'deny'
     * @returns {Promise<object>} - Resolves with { succeeded: [...], failed: [...] }
     */
    function bulkNoteAction(commitId, noteIds, action) {
        return apiCall(`/BulkNoteAction/${commitId}`, 'POST', {
            note_ids: noteIds.map(id => parseInt(id, 10)),
            action: action
        });
    }

    // -- Image Loading --
    /**
     * Fetches a presigned URL for a given filename and context.
     * @param {string} filename
     * @param {string} contextType - e.g., 'note'
     * @param {string} contextId - e.g., the note's ID
     * @returns {Promise<object>} - Resolves with { success: true, presigned_url: "..." } or rejects.
     */
    function getPresignedImageUrl(filename, contextType, contextId) {
        if (!contextType || !contextId) {
             console.error("Missing contextType or contextId for getPresignedImageUrl");
             return Promise.reject(new Error("Missing context for image URL fetch."));
        }
        return apiCall('/GetImageFile', 'POST', {
            filename: filename,
            context_type: contextType,
            context_id: contextId
        });
    }


    return {
        // Action methods
        acceptTag,
        denyTag,
        acceptMove,
        denyMove,
        acceptField,
        denyField,
        acceptNote,
        deleteNote,
        acceptNoteRemoval,
        denyNoteRemoval,
        updateFieldSuggestion,
        // Edit All Fields
        getAllFieldsForEdit,
        batchUpdateFieldSuggestions,
        // Bulk note actions
        bulkNoteAction,
        // Image methods
        getPresignedImageUrl,
        // Utility methods
        getContext,
        getLastResponseMeta: () => lastResponseMeta,
        // Expose apiCall if needed directly elsewhere, but generally prefer specific methods
        apiCall
    };
})();